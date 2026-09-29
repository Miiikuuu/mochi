//! Runtime paths for the relocatable /usr/libexec/mochi installation.
use std::path::PathBuf;

pub fn installed_root() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let root = executable.parent()?.parent()?;
    root.join("installation.json")
        .is_file()
        .then(|| root.to_path_buf())
}

pub fn bubble_host() -> PathBuf {
    installed_root().map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../platform/linux/bubble_host.py"),
        |root| root.join("platform/linux/bubble_host.py"),
    )
}

pub fn default_packages() -> Vec<PathBuf> {
    if let Some(root) = installed_root() {
        return vec![root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("share/mochi/characters/mochi-dot.mochi")];
    }
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if source_root
        .join("PUBLIC_SOURCE_DISTRIBUTION.json")
        .is_file()
    {
        return vec![source_root.join("characters/mochi-dot.mochi")];
    }
    let root = source_root.join("characters/lime-squad-png");
    ["angelina_wish", "shanbi", "jacinta", "timeslot"]
        .map(|key| root.join(format!("{key}.mochi")))
        .to_vec()
}
