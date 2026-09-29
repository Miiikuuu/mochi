//! Versioned data contracts. No platform or execution interfaces.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError(pub String);
impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for SchemaError {}

/// At least two dot-separated lowercase ASCII segments, e.g. `agent.thinking`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Namespace(String);
impl Namespace {
    pub fn new(value: impl Into<String>) -> Result<Self, SchemaError> {
        let value = value.into();
        let valid = value.len() <= 128
            && value.contains('.')
            && value.split('.').all(|part| {
                part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                    && part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
            });
        if !valid {
            return Err(SchemaError(format!("invalid namespace: {value}")));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for Namespace {
    type Error = SchemaError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<Namespace> for String {
    fn from(value: Namespace) -> Self {
        value.0
    }
}
impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Portable data; deliberately no executable, OS handle, TOML datetime or null.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}
impl Value {
    pub fn validate(&self) -> Result<(), SchemaError> {
        self.validate_at(0)
    }
    fn validate_at(&self, depth: usize) -> Result<(), SchemaError> {
        if depth > 32 {
            return Err(SchemaError("value nesting exceeds 32".into()));
        }
        match self {
            Self::Float(n) if !n.is_finite() => Err(SchemaError("non-finite number".into())),
            Self::Array(values) => values.iter().try_for_each(|v| v.validate_at(depth + 1)),
            Self::Object(values) => values.values().try_for_each(|v| v.validate_at(depth + 1)),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Component {
    pub id: Namespace,
    #[serde(default)]
    pub config: BTreeMap<String, Value>,
}

/// Requested scope metadata, never an authorization grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permission {
    pub capability: Namespace,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ManifestWire")]
pub struct MateManifest {
    pub schema_version: u32,
    pub id: Namespace,
    pub name: String,
    pub version: semver::Version,
    pub components: Vec<Component>,
    pub capabilities: BTreeSet<Namespace>,
    pub permissions: Vec<Permission>,
}

// Keep the versioned wire boundary separate so a future decoder can migrate
// a supported historical representation explicitly before constructing this type.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    schema_version: u32,
    id: Namespace,
    name: String,
    version: semver::Version,
    components: Vec<Component>,
    #[serde(default)]
    capabilities: BTreeSet<Namespace>,
    #[serde(default)]
    permissions: Vec<Permission>,
}
impl TryFrom<ManifestWire> for MateManifest {
    type Error = SchemaError;
    fn try_from(w: ManifestWire) -> Result<Self, Self::Error> {
        let value = Self {
            schema_version: w.schema_version,
            id: w.id,
            name: w.name,
            version: w.version,
            components: w.components,
            capabilities: w.capabilities,
            permissions: w.permissions,
        };
        value.validate()?;
        Ok(value)
    }
}
impl MateManifest {
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(SchemaError(format!(
                "unsupported schema_version {}; supported: {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(SchemaError("name must contain 1..=256 bytes".into()));
        }
        let mut ids = BTreeSet::new();
        for component in &self.components {
            if !ids.insert(&component.id) {
                return Err(SchemaError(format!("duplicate component {}", component.id)));
            }
            for value in component.config.values() {
                value.validate()?;
            }
        }
        let mut permissions = BTreeSet::new();
        for permission in &self.permissions {
            if !self.capabilities.contains(&permission.capability) {
                return Err(SchemaError(format!(
                    "permission for undeclared capability {}",
                    permission.capability
                )));
            }
            if permission.scope.trim().is_empty() || permission.scope.len() > 1024 {
                return Err(SchemaError(
                    "permission scope must contain 1..=1024 bytes".into(),
                ));
            }
            if !permissions.insert((&permission.capability, &permission.scope)) {
                return Err(SchemaError("duplicate permission".into()));
            }
        }
        Ok(())
    }
    pub fn from_toml(input: &str) -> Result<Self, SchemaError> {
        // Decode version before v1 fields: future versions never silently use v1.
        let raw: toml::Value = toml::from_str(input).map_err(|e| SchemaError(e.to_string()))?;
        if raw.get("schema_version").and_then(toml::Value::as_integer)
            != Some(SCHEMA_VERSION as i64)
        {
            return Err(SchemaError(
                "missing or unsupported schema_version; supported: 1".into(),
            ));
        }
        raw.try_into()
            .map_err(|e: toml::de::Error| SchemaError(e.to_string()))
    }
    pub fn to_toml(&self) -> Result<String, SchemaError> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|e| SchemaError(e.to_string()))
    }
}
