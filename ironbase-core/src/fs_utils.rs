//! Filesystem helpers for durable operations.

use std::path::Path;

use crate::error::Result;

/// Atomically rename `src` to `dst`, then fsync the parent directory.
///
/// POSIX `rename()` is atomic at the filesystem level, but the directory entry
/// update is not guaranteed durable until the parent directory is fsynced.
/// Without this, a crash after the rename can lose the directory entry on
/// filesystems that defer metadata writes. The directory fsync is skipped on
/// non-Unix platforms (NTFS journals renames). Directory fsync failures are
/// logged as warnings but not propagated — the rename itself has succeeded.
pub fn atomic_rename_and_sync<S: AsRef<Path>, D: AsRef<Path>>(src: S, dst: D) -> Result<()> {
    let dst = dst.as_ref();
    std::fs::rename(src.as_ref(), dst)?;

    #[cfg(unix)]
    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            match std::fs::File::open(parent) {
                Ok(dir) => {
                    if let Err(e) = dir.sync_all() {
                        tracing::warn!(
                            error = %e,
                            path = ?parent,
                            "atomic_rename_and_sync: parent dir fsync failed (non-fatal)"
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    error = %e,
                    path = ?parent,
                    "atomic_rename_and_sync: parent dir open failed (non-fatal)"
                ),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rename_persists_and_new_path_exists() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.tmp");
        let dst = dir.path().join("a.final");

        std::fs::write(&src, b"hello").unwrap();
        atomic_rename_and_sync(&src, &dst).unwrap();

        assert!(!src.exists());
        assert!(dst.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
    }

    #[test]
    fn rename_missing_src_returns_error() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("does_not_exist.tmp");
        let dst = dir.path().join("final");

        let result = atomic_rename_and_sync(&src, &dst);
        assert!(result.is_err());
    }
}
