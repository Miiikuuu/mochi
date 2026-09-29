use crate::*;
use mochi_schema::MateManifest;

#[test]
fn manifest_toml_round_trip_preserves_extensions_and_permissions() {
    let mut manifest = MateManifest::from_toml(&manifest_text()).unwrap();
    manifest.capabilities.insert(ns("host.notify"));
    manifest.permissions.push(mochi_schema::Permission {
        capability: ns("host.notify"),
        scope: "desktop".into(),
    });
    manifest.components.push(mochi_schema::Component {
        id: ns("future.component"),
        config: BTreeMap::from([
            ("enabled".into(), Value::Bool(true)),
            (
                "nested".into(),
                Value::Array(vec![Value::Integer(3), Value::String("hello".into())]),
            ),
            ("amount".into(), Value::Float(0.5)),
        ]),
    });
    assert_eq!(
        MateManifest::from_toml(&manifest.to_toml().unwrap()).unwrap(),
        manifest
    );
}

#[test]
fn invalid_schema_is_rejected_even_by_direct_serde_decode() {
    for bad in ["0", "2", "-1", "1.5", "\"1\""] {
        let text =
            manifest_text().replace("schema_version = 1", &format!("schema_version = {bad}"));
        assert!(MateManifest::from_toml(&text).is_err(), "{bad}");
        assert!(toml::from_str::<MateManifest>(&text).is_err(), "{bad}");
    }
    assert!(MateManifest::from_toml(&manifest_text().replace("schema_version = 1", "")).is_err());
}

#[test]
fn invalid_names_version_unknown_fields_and_duplicate_components_are_rejected() {
    for text in [
        manifest_text().replace("mochi.neko", "neko"),
        manifest_text().replace("name = \"Neko\"", "name = \" \""),
        manifest_text().replace("version = \"0.1.0\"", "version = \"banana\""),
        format!("unrecognized = true\n{}", manifest_text()),
        format!(
            "{}\n[[components]]\nid = \"mochi.behavior.wander\"",
            manifest_text()
        ),
    ] {
        assert!(MateManifest::from_toml(&text).is_err());
    }
}

#[test]
fn undeclared_permissions_and_nonfinite_config_fail_validation() {
    let mut manifest = MateManifest::from_toml(&manifest_text()).unwrap();
    manifest.permissions.push(mochi_schema::Permission {
        capability: ns("host.shell"),
        scope: "*".into(),
    });
    assert!(manifest.validate().is_err());
    manifest.permissions.clear();
    manifest.components[0]
        .config
        .insert("bad".into(), Value::Float(f64::NAN));
    assert!(manifest.to_toml().is_err());
}

#[test]
fn namespace_and_value_validation() {
    for name in [
        "agent.thinking",
        "git.dirty",
        "ros2.node_offline",
        "org.mochi.some-key",
    ] {
        assert!(Namespace::new(name).is_ok());
    }
    for name in ["", "hello", ".a", "a.", "a..b", "A.b", "a/b.c", "a.💖"] {
        assert!(Namespace::new(name).is_err());
    }
    let mut value = Value::Bool(true);
    for _ in 0..34 {
        value = Value::Array(vec![value]);
    }
    assert!(value.validate().is_err());
}
