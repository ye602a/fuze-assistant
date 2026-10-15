//! Bounded, update-time retirement of immutable packages. Never runs in the frame loop.
use crate::Result;
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Unknown files, links and excessive trees are preserved. Delete files individually,
/// then empty directories: never recursively follow a user-controlled directory.
pub fn retire(root: &Path, target: &Path, allowed: &dyn Fn(&str) -> bool) -> Result<()> {
    process(root, target, allowed, false)
}
pub fn inspect(root: &Path, target: &Path, allowed: &dyn Fn(&str) -> bool) -> Result<()> {
    process(root, target, allowed, true)
}
fn process(
    root: &Path,
    target: &Path,
    allowed: &dyn Fn(&str) -> bool,
    inspect_only: bool,
) -> Result<()> {
    let root = fs::canonicalize(root).map_err(|_| "cleanup-root")?;
    validate(&root, target)?;
    let target = fs::canonicalize(target).map_err(|_| "cleanup-target")?;
    if target.parent() != Some(root.as_path()) {
        return Err("cleanup-boundary");
    }
    let mut files = Vec::new();
    let mut dirs = vec![target.clone()];
    let mut index = 0;
    while index < dirs.len() {
        let dir = dirs[index].clone();
        validate(&root, &dir)?;
        for entry in fs::read_dir(&dir).map_err(|_| "cleanup-read")?.take(4097) {
            let path = entry.map_err(|_| "cleanup-read")?.path();
            validate(&root, &path)?;
            let metadata = fs::symlink_metadata(&path).map_err(|_| "cleanup-read")?;
            if metadata.is_dir() {
                dirs.push(path);
                if dirs.len() > 64 {
                    return Err("cleanup-limit");
                }
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(&target)
                    .map_err(|_| "cleanup-boundary")?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !allowed(&relative) {
                    return Err("cleanup-unknown-file");
                }
                files.push(path);
                if files.len() > 4096 {
                    return Err("cleanup-limit");
                }
            } else {
                return Err("cleanup-file-kind");
            }
        }
        index += 1;
    }
    if inspect_only {
        return Ok(());
    }
    for path in files {
        validate(&root, &path)?;
        fs::remove_file(path).map_err(|_| "cleanup-remove")?;
    }
    for path in dirs.into_iter().rev() {
        validate(&root, &path)?;
        fs::remove_dir(path).map_err(|_| "cleanup-remove")?;
    }
    Ok(())
}
fn validate(root: &Path, path: &Path) -> Result<()> {
    if !path.starts_with(root) || path == root {
        return Err("cleanup-boundary");
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| "cleanup-read")?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("cleanup-reparse");
        }
    }
    if fs::canonicalize(path).map_err(|_| "cleanup-read")? != path {
        return Err("cleanup-boundary");
    }
    Ok(())
}
pub fn children(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .take(65)
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_unknown_files_and_outside_targets() {
        let root = std::env::temp_dir().join(format!(
            "fuze-retire-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("old/sub")).unwrap();
        fs::write(root.join("old/sub/data.json"), b"{}").unwrap();
        fs::write(root.join("old/personal.txt"), b"fixture").unwrap();
        let root = fs::canonicalize(root).unwrap();
        assert_eq!(
            retire(&root, &root.join("old"), &|s| s.ends_with(".json")),
            Err("cleanup-unknown-file")
        );
        assert!(root.join("old/sub/data.json").exists());
        assert_eq!(retire(&root, &root, &|_| true), Err("cleanup-boundary"));
        fs::remove_file(root.join("old/personal.txt")).unwrap();
        retire(&root, &root.join("old"), &|s| s.ends_with(".json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
