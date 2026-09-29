use std::path::PathBuf;

#[test]
fn core_and_runtime_dependencies_cannot_point_back_to_platform_or_package() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for (name, allowed) in [
        ("mochi-core", vec!["mochi-schema"]),
        ("mochi-runtime", vec!["mochi-core"]),
    ] {
        let text =
            std::fs::read_to_string(root.join("crates").join(name).join("Cargo.toml")).unwrap();
        let manifest: toml::Value = toml::from_str(&text).unwrap();
        for dependency in manifest["dependencies"].as_table().unwrap().keys() {
            assert!(
                allowed.contains(&dependency.as_str()),
                "{name} gained dependency {dependency}; review architectural direction"
            );
        }
        assert!(
            manifest.get("target").is_none(),
            "review target-specific dependencies in {name}"
        );
    }
}

#[test]
fn host_and_renderer_remain_outer_independent_adapters() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates");
    for name in [
        "mochi-schema",
        "mochi-core",
        "mochi-runtime",
        "mochi-package",
        "mochi-platform",
        "mochi-broker",
        "mochi-renderer-software",
        "mochi-host",
    ] {
        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(root.join(name).join("Cargo.toml")).unwrap())
                .unwrap();
        let deps = manifest["dependencies"].as_table().unwrap();
        assert!(
            !deps.contains_key("mochi-host"),
            "{name} must not depend on Host"
        );
        assert!(
            !deps.contains_key("mochi-renderer-software"),
            "{name} must not depend on a concrete renderer"
        );
        assert!(
            manifest.get("target").is_none(),
            "review platform dependencies in {name}"
        );
    }
}

#[test]
fn window_libraries_are_confined_to_the_preview_executable() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates");
    for entry in std::fs::read_dir(root).unwrap() {
        let manifest: toml::Value = toml::from_str(
            &std::fs::read_to_string(entry.unwrap().path().join("Cargo.toml")).unwrap(),
        )
        .unwrap();
        let deps = manifest["dependencies"].as_table().unwrap();
        for name in ["winit", "softbuffer"] {
            assert!(
                !deps.contains_key(name),
                "window dependency escaped into a library crate"
            );
        }
        assert!(
            manifest.get("target").is_none(),
            "review target-specific library dependencies"
        );
    }
}
