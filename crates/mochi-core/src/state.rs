use crate::{Namespace, SchemaError, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}
impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn validate(self) -> Result<(), SchemaError> {
        if self.x.is_finite() && self.y.is_finite() {
            Ok(())
        } else {
            Err(SchemaError("coordinates must be finite".into()))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MateState {
    pub position: Vec2,
    pub velocity: Vec2,
    pub current_animation: String,
    pub current_state: String,
    pub values: BTreeMap<Namespace, Value>,
}
impl Default for MateState {
    fn default() -> Self {
        Self {
            position: Vec2::default(),
            velocity: Vec2::default(),
            current_animation: "idle".into(),
            current_state: "idle".into(),
            values: BTreeMap::new(),
        }
    }
}
impl MateState {
    pub fn validate(&self) -> Result<(), SchemaError> {
        self.position.validate()?;
        self.velocity.validate()?;
        validate_label(&self.current_animation)?;
        validate_label(&self.current_state)?;
        self.values.values().try_for_each(Value::validate)
    }
}
pub(crate) fn validate_label(value: &str) -> Result<(), SchemaError> {
    if value.trim().is_empty() || value.len() > 256 {
        Err(SchemaError(
            "state/animation label must contain 1..=256 bytes".into(),
        ))
    } else {
        Ok(())
    }
}
