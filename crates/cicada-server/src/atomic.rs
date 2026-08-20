//! Atomic file replacement — the ONE way the server writes a project file
//! (the `.cic`, its layout sidecar, `scripts/*.py`): a sibling temp file,
//! `sync_all`, then a rename over the target. A crash or a concurrent
//! reader (git, the watcher, an editor, a sync client) never sees a
//! truncated source of truth, and a failed rename leaves the target as it
//! was.

use std::path::Path;

/// Write `bytes` to `path` atomically. The temp file is
/// `.<name>.cicada-tmp` beside the target; it is removed again when the
/// rename fails.
///
/// # Errors
///
/// The first I/O failure (create / write / sync / rename). On a rename
/// failure the target is untouched.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.cicada-tmp"));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_rename_leaves_the_target_and_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.txt");
        std::fs::write(&target, b"before").unwrap();
        // A non-empty directory at the target: the rename cannot replace it
        // on any platform.
        let blocked = dir.path().join("blocked");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("x"), b"x").unwrap();
        assert!(write_atomic(&blocked, b"new").is_err());
        assert!(blocked.is_dir(), "the target is untouched");
        assert!(
            !dir.path().join(".blocked.cicada-tmp").exists(),
            "the temp file was removed"
        );
        // The happy path replaces the bytes and leaves no temp file.
        write_atomic(&target, b"after").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"after");
        assert!(!dir.path().join(".t.txt.cicada-tmp").exists());
    }
}
