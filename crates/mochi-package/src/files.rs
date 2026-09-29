use crate::{error, PackageError};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOML_BYTES: usize = 64 * 1024;

pub(crate) fn validate_asset_key(key: &str) -> Result<(), PackageError> {
    if key.len() > 256
        || key.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        })
    {
        return Err(error(format!("invalid package-relative path: {key}")));
    }
    Ok(())
}
fn allowed_file(key: &str) -> bool {
    if matches!(
        key,
        "manifest.toml" | "behavior.toml" | "animations.toml" | "rules.toml" | "spine.toml"
    ) {
        return true;
    }
    let extension = key.rsplit('.').next().unwrap_or("");
    (key.starts_with("sprites/") && matches!(extension, "png" | "jpg" | "jpeg" | "webp"))
        || (key.starts_with("spine/") && matches!(extension, "skel" | "atlas" | "json"))
        || (key.starts_with("sounds/") && matches!(extension, "wav" | "ogg"))
}

pub(crate) fn snapshot(
    root: &Path,
    max_files: usize,
) -> Result<BTreeMap<String, Vec<u8>>, PackageError> {
    if root.extension().and_then(|s| s.to_str()) != Some("mochi") {
        return Err(error("package must be a .mochi directory"));
    }
    // The host selects the parent directory. Resolve the package root and every
    // child through held directory handles, never by rebuilding ambient paths.
    let parent_path = root
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = root
        .file_name()
        .ok_or_else(|| error("missing package directory name"))?;
    let parent = Dir::open_ambient_dir(parent_path, cap_std::ambient_authority()).map_err(error)?;
    let directory = parent.open_dir_nofollow(name).map_err(error)?;
    let mut output = BTreeMap::new();
    let mut total = 0;
    let mut entries = 0;
    walk(
        &directory,
        "",
        0,
        &mut total,
        &mut entries,
        &mut output,
        max_files,
    )?;
    Ok(output)
}
fn walk(
    root: &Dir,
    prefix: &str,
    depth: usize,
    total: &mut usize,
    entries: &mut usize,
    output: &mut BTreeMap<String, Vec<u8>>,
    max_files: usize,
) -> Result<(), PackageError> {
    if depth > 4 {
        return Err(error("package nesting exceeds 4 directories"));
    }
    let mut children = root
        .entries()
        .map_err(error)?
        .take(max_files * 2 + 1)
        .collect::<Result<Vec<_>, _>>()
        .map_err(error)?;
    // Bound enumeration itself, including directories that contain no files.
    if children.len() > max_files * 2 {
        return Err(error("too many directory entries"));
    }
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        *entries += 1;
        if *entries > max_files * 2 {
            return Err(error("package entry limit exceeded"));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("package filenames must be ASCII"))?;
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        validate_asset_key(&key)?;
        let meta = root.symlink_metadata(&name).map_err(error)?;
        if meta.file_type().is_symlink() {
            return Err(error(format!("symlinks are forbidden: {key}")));
        }
        if meta.is_dir() {
            if depth == 0 && key != "sprites" && key != "sounds" && key != "spine" {
                return Err(error(format!("unsupported directory: {key}")));
            }
            let child = root.open_dir_nofollow(&name).map_err(error)?;
            walk(&child, &key, depth + 1, total, entries, output, max_files)?;
        } else {
            if !meta.is_file() || !allowed_file(&key) {
                return Err(error(format!("unsupported package file: {key}")));
            }
            if output.len() >= max_files {
                return Err(error("package file limit exceeded"));
            }
            let limit = if key.ends_with(".toml") {
                MAX_TOML_BYTES
            } else {
                MAX_FILE_BYTES
            };
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No).nonblock(true);
            let mut file = root.open_with(&name, &options).map_err(error)?;
            // Check the opened object, not only the earlier directory entry.
            let before = file.metadata().map_err(error)?;
            check_regular(&before, &key)?;
            if before.len() > limit as u64 {
                return Err(error(format!("file too large: {key}")));
            }
            let read_limit = limit.min(MAX_TOTAL_BYTES - *total);
            if before.len() > read_limit as u64 {
                return Err(error("package byte limit exceeded"));
            }
            let mut data = Vec::new();
            (&mut file)
                .take(read_limit as u64 + 1)
                .read_to_end(&mut data)
                .map_err(error)?;
            let after = file.metadata().map_err(error)?;
            check_regular(&after, &key)?;
            if before.len() != data.len() as u64 || !unchanged(&before, &after)? {
                return Err(error(format!("file changed during import: {key}")));
            }
            if data.len() > limit {
                return Err(error(format!("file too large: {key}")));
            }
            *total += data.len();
            if *total > MAX_TOTAL_BYTES {
                return Err(error("package byte limit exceeded"));
            }
            output.insert(key, data);
        }
    }
    Ok(())
}

fn check_regular(meta: &Metadata, key: &str) -> Result<(), PackageError> {
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(error(format!(
            "resource must be a regular file with one link: {key}"
        )));
    }
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            return Err(error(format!("executable file is forbidden: {key}")));
        }
    }
    Ok(())
}

fn unchanged(before: &Metadata, after: &Metadata) -> Result<bool, PackageError> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as UnixMetadataExt;
        if before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec() {
            return Ok(false);
        }
    }
    Ok(before.len() == after.len()
        && before.modified().map_err(error)? == after.modified().map_err(error)?)
}
