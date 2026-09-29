use crate::*;
use mochi_package::LoadedPackage;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) struct Fixture(pub(super) PathBuf);
impl Fixture {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mochi-test-{}-{}.mochi",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("sprites")).unwrap();
        for file in ["manifest.toml", "behavior.toml", "sprites/neko.png"] {
            fs::copy(neko_path().join(file), path.join(file)).unwrap();
        }
        Self(path)
    }
    pub(super) fn replace(&self, file: &str, from: &str, to: &str) {
        let path = self.0.join(file);
        fs::write(&path, fs::read_to_string(&path).unwrap().replace(from, to)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn package_loads_without_ai_or_capabilities() {
    let package = LoadedPackage::load(neko_path()).unwrap();
    assert!(package.manifest().capabilities.is_empty());
    assert!(package.manifest().permissions.is_empty());
    assert_eq!(package.assets().len(), 1);
    assert!(package.assets()["sprites/neko.png"].starts_with(b"\x89PNG\r\n\x1a\n"));
    package.instantiate("neko").unwrap();
}

#[test]
fn file_budget_is_host_selected_and_keeps_decode_limits() {
    use mochi_package::{DecodeLimits, LoadOptions};
    let fixture = Fixture::new();
    for index in 0..126 {
        fs::copy(
            fixture.0.join("sprites/neko.png"),
            fixture.0.join(format!("sprites/copy-{index}.png")),
        )
        .unwrap();
    }
    assert!(LoadedPackage::load(&fixture.0)
        .err()
        .unwrap()
        .to_string()
        .contains("file limit"));
    let options = LoadOptions {
        max_files: 129,
        ..LoadOptions::default()
    };
    let package = LoadedPackage::load_with_options(&fixture.0, options).unwrap();
    assert_eq!(package.assets().len(), 127);
    let same = LoadedPackage::load_with_options(
        &fixture.0,
        LoadOptions {
            max_files: 256,
            ..options
        },
    )
    .unwrap();
    assert_eq!(package.content_hash(), same.content_hash());
    assert!(LoadedPackage::load_with_options(
        &fixture.0,
        LoadOptions {
            decode: DecodeLimits {
                max_total_image_bytes: 0,
                ..DecodeLimits::default()
            },
            ..options
        }
    )
    .is_err());
    for max_files in [0, 128, 4097, usize::MAX] {
        assert!(LoadedPackage::load_with_options(
            &fixture.0,
            LoadOptions {
                max_files,
                ..options
            }
        )
        .is_err());
    }
}

#[test]
fn configurable_file_budget_also_bounds_empty_directory_enumeration() {
    use mochi_package::LoadOptions;
    let fixture = Fixture::new();
    for index in 0..300 {
        fs::create_dir(fixture.0.join(format!("sprites/empty-{index}"))).unwrap();
    }
    assert!(LoadedPackage::load(&fixture.0).is_err());
    LoadedPackage::load_with_options(
        &fixture.0,
        LoadOptions {
            max_files: 256,
            ..LoadOptions::default()
        },
    )
    .unwrap();
    for index in 300..512 {
        fs::create_dir(fixture.0.join(format!("sprites/empty-{index}"))).unwrap();
    }
    assert!(LoadedPackage::load_with_options(
        &fixture.0,
        LoadOptions {
            max_files: 256,
            ..LoadOptions::default()
        }
    )
    .is_err());
}

#[test]
fn assets_are_snapshotted_in_memory_not_reopened_after_load() {
    let fixture = Fixture::new();
    let package = LoadedPackage::load(&fixture.0).unwrap();
    let bytes = package.assets()["sprites/neko.png"].clone();
    fs::remove_file(fixture.0.join("sprites/neko.png")).unwrap();
    assert_eq!(package.assets()["sprites/neko.png"], bytes);
    package.instantiate("still-works").unwrap();
}

#[test]
fn package_can_choose_builtin_idle_component() {
    let fixture = Fixture::new();
    fixture.replace(
        "manifest.toml",
        "mochi.behavior.wander",
        "mochi.behavior.idle",
    );
    fixture.replace("manifest.toml", "file = \"behavior.toml\"", "");
    fs::remove_file(fixture.0.join("behavior.toml")).unwrap();
    let mut mate = LoadedPackage::load(&fixture.0)
        .unwrap()
        .instantiate("idle")
        .unwrap();
    assert!(mate
        .process(MateEvent::Tick {
            delta: std::time::Duration::from_secs(1)
        })
        .unwrap()
        .trace[0]
        .actions
        .is_empty());
}

#[test]
fn scripts_and_shared_libraries_are_rejected() {
    for filename in [
        "run.sh",
        "main.py",
        "main.js",
        "plugin.so",
        "plugin.dll",
        "plugin.dylib",
        "plugin.wasm",
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.0.join(filename), b"untrusted bytes").unwrap();
        assert!(LoadedPackage::load(&fixture.0).is_err(), "{filename}");
    }
}

#[test]
fn unknown_component_is_preserved_by_schema_but_rejected_by_loader() {
    let fixture = Fixture::new();
    fixture.replace("manifest.toml", "mochi.behavior.wander", "future.custom");
    assert!(mochi_schema::MateManifest::from_toml(
        &fs::read_to_string(fixture.0.join("manifest.toml")).unwrap()
    )
    .is_ok());
    let error = LoadedPackage::load(&fixture.0)
        .err()
        .expect("expected error");
    assert!(error.to_string().contains("unsupported component"));
}

#[test]
fn traversal_absolute_paths_and_missing_assets_are_rejected() {
    for bad in [
        "../secret.png",
        "/tmp/secret.png",
        "sprites/../secret.png",
        "sprites\\secret.png",
        "sprites/missing.png",
        "https://example.com/a.png",
    ] {
        let fixture = Fixture::new();
        // TOML literal string keeps backslash input valid for the path validator.
        fixture.replace("manifest.toml", "\"sprites/neko.png\"", &format!("'{bad}'"));
        assert!(LoadedPackage::load(&fixture.0).is_err(), "{bad}");
    }
}

#[test]
fn malformed_behavior_unknown_config_and_missing_required_files_are_rejected() {
    for (file, from, to) in [
        ("behavior.toml", "speed = 10.0", "speed = -1.0"),
        ("behavior.toml", "schema_version = 1", "schema_version = 2"),
        ("behavior.toml", "speed = 10.0", "speed = nan"),
        ("manifest.toml", "width = 32.0", "width = 0.0"),
        (
            "manifest.toml",
            "width = 32.0",
            "width = 32.0\nscript = 'bad'",
        ),
        ("manifest.toml", "behavior.toml", "../behavior.toml"),
    ] {
        let fixture = Fixture::new();
        fixture.replace(file, from, to);
        assert!(LoadedPackage::load(&fixture.0).is_err(), "{file} {to}");
    }
    for file in ["manifest.toml", "behavior.toml"] {
        let fixture = Fixture::new();
        fs::remove_file(fixture.0.join(file)).unwrap();
        assert!(LoadedPackage::load(&fixture.0).is_err());
    }
}

#[test]
fn oversized_files_and_excessive_file_counts_are_rejected() {
    let fixture = Fixture::new();
    fs::File::create(fixture.0.join("sprites/large.png"))
        .unwrap()
        .set_len(8 * 1024 * 1024 + 1)
        .unwrap();
    assert!(LoadedPackage::load(&fixture.0).is_err());
    fs::remove_file(fixture.0.join("sprites/large.png")).unwrap();
    for i in 0..130 {
        fs::write(fixture.0.join(format!("sprites/{i}.png")), []).unwrap();
    }
    assert!(LoadedPackage::load(&fixture.0).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_and_executable_assets_are_rejected() {
    use std::os::unix::{fs::symlink, fs::PermissionsExt};
    let fixture = Fixture::new();
    symlink("neko.png", fixture.0.join("sprites/link.png")).unwrap();
    assert!(LoadedPackage::load(&fixture.0).is_err());
    fs::remove_file(fixture.0.join("sprites/link.png")).unwrap();
    fs::set_permissions(
        fixture.0.join("sprites/neko.png"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(LoadedPackage::load(&fixture.0).is_err());
}

#[test]
fn package_permissions_do_not_enable_execution() {
    let fixture = Fixture::new();
    fixture.replace(
        "manifest.toml",
        "capabilities = []",
        "capabilities = ['host.shell']",
    );
    fixture.replace(
        "manifest.toml",
        "permissions = []",
        "permissions = [{ capability = 'host.shell', scope = '*' }]",
    );
    let package = LoadedPackage::load(&fixture.0).unwrap();
    let mut mate = package.instantiate("unprivileged").unwrap();
    let output = mate
        .process(MateEvent::Tick {
            delta: std::time::Duration::from_secs(1),
        })
        .unwrap();
    assert!(output.capability_requests.is_empty());
    assert_eq!(output.state.position.x, 10.0);
}
