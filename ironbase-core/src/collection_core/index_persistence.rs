use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::fulltext::{FulltextIndex, FulltextIndexMetadata};
use crate::index::fuzzy::{FuzzyIndex, FuzzyIndexMetadata};
use crate::index::{BPlusTree, IndexMetadata};

pub(crate) fn sanitize_component(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "index".to_string()
    } else {
        sanitized
    }
}

/// Build an index file path with a given extension.
///
/// All index types use the same naming scheme: `{db_stem}_{sanitized_name}_{hash}.{ext}`
fn build_index_file_path_with_ext(
    db_file_path: &str,
    index_name: &str,
    extension: &str,
) -> Option<PathBuf> {
    if db_file_path.is_empty() {
        return None;
    }

    let base_path = Path::new(db_file_path);
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("database");

    let safe_component = sanitize_component(index_name);
    let mut hasher = DefaultHasher::new();
    index_name.hash(&mut hasher);
    let hash = hasher.finish();

    // Use full 64-bit hash to minimize collision risk with many indexes
    let file_name = format!("{}_{}_{:016x}.{}", stem, safe_component, hash, extension);
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(parent.join(file_name))
}

fn build_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
    build_index_file_path_with_ext(db_file_path, index_name, "idx")
}

/// Build the .idx file path for a B+ tree index (public for drop_collection cleanup)
pub fn build_btree_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
    build_index_file_path_with_ext(db_file_path, index_name, "idx")
}

/// Build the .hnsw cache file path for a vector index
pub fn build_vector_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
    build_index_file_path_with_ext(db_file_path, index_name, "hnsw")
}

/// Extensions of the index cache files (each also with a `.tmp` suffix
/// while it is being written)
const INDEX_CACHE_EXTENSIONS: [&str; 4] = ["idx", "ftidx", "fzidx", "hnsw"];

/// True if no other IronBase database (a file starting with the `.mlite`
/// magic) shares the directory of `db_path`. Vector caches written before
/// they carried the database name (`{index}.hnsw`) belong to the database
/// only then: two databases in one directory shared them.
pub(crate) fn is_only_database_in_dir(db_path: &Path) -> bool {
    use std::io::Read;
    let parent = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name() == db_path.file_name() || !path.is_file() {
            continue;
        }
        let mut magic = [0u8; 8];
        let is_database = File::open(&path)
            .and_then(|mut f| f.read_exact(&mut magic))
            .is_ok()
            && &magic == b"MONGOLTE";
        if is_database {
            return false;
        }
    }
    true
}

/// Every index cache file of the database at `db_path` that exists:
/// `{db_stem}_*.{idx,ftidx,fzidx,hnsw}` (and their `.tmp` files), plus the
/// `{index}.hnsw` vector caches of earlier versions when no other database
/// shares the directory.
///
/// The caches describe one state of the database. A database file put back
/// from a backup or a copy must not be opened next to the caches of another
/// state (P10, 2026-10-28); deleting them makes the next open rebuild the
/// indexes from the documents. A prefix also matches a database whose name
/// extends this one's (`db` and `db_old`): its caches are rebuilt too.
pub fn index_cache_files(db_path: &Path) -> Vec<PathBuf> {
    let parent = match db_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let stem = db_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("database");
    let prefix = format!("{}_", stem);
    let legacy_vector_caches = is_only_database_in_dir(db_path);
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = name.strip_suffix(".tmp").unwrap_or(name);
        let Some((_, ext)) = name.rsplit_once('.') else {
            continue;
        };
        let ours = (name.starts_with(&prefix) && INDEX_CACHE_EXTENSIONS.contains(&ext))
            || (legacy_vector_caches && ext == "hnsw");
        if ours && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files
}

pub fn persist_index_to_disk<F, T>(db_file_path: &str, index_name: &str, save_fn: F) -> Result<()>
where
    F: FnOnce(&mut File) -> Result<T>,
{
    if let Some(index_file_path) = build_index_file_path(db_file_path, index_name) {
        // Write a temp file and rename it over the index: a lazy-mode index
        // still reads its unloaded nodes from the current file while it is
        // saved, so truncating that file first destroyed the index (audit
        // 2026-10-06 #22). The rename also makes a crash mid-save harmless.
        let temp_path = index_file_path.with_extension("idx.tmp");
        // A leftover temp file is stale (one writer per database). Remove it
        // and create the temp file exclusively: O_EXCL never follows a
        // symlink, so a planted `.idx.tmp` link cannot redirect the write or
        // truncate its target.
        match std::fs::remove_file(&temp_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        save_fn(&mut file)?;
        file.sync_all()?;
        drop(file);
        crate::fs_utils::atomic_rename_and_sync(&temp_path, &index_file_path)?;
    }
    Ok(())
}

/// Try to load an index from .idx file (graceful degradation)
/// Returns None if file doesn't exist or is corrupted (fallback to rebuild)
///
/// Uses lazy loading: only root node is loaded, children are loaded on-demand.
pub fn try_load_index_from_file(
    db_file_path: &str,
    index_meta: &IndexMetadata,
) -> Option<BPlusTree> {
    let idx_path = build_index_file_path(db_file_path, &index_meta.name)?;

    if !idx_path.exists() {
        return None;
    }

    // A file from before exact float parsing (0.3.437) may hold float keys
    // 1 ULP off the stored values: rebuild it from the documents once (the
    // rebuilt index is marked dirty and written back with the new footer).
    // `_id` keys are never floats.
    if index_meta.field != "_id" && !BPlusTree::has_exact_float_keys(&idx_path) {
        crate::log_info!(
            "Index '{}' predates exact float keys: rebuilding from documents",
            index_meta.name
        );
        return None;
    }

    // Use lazy loading - only loads root node, children loaded on-demand
    BPlusTree::load_from_path(idx_path, index_meta.clone()).ok()
}

// ============================================================================
// Fulltext Index Persistence
// ============================================================================

/// Build the .ftidx file path for a fulltext index
pub fn build_fulltext_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
    build_index_file_path_with_ext(db_file_path, index_name, "ftidx")
}

/// Try to load a fulltext index from .ftidx file (graceful degradation)
/// Returns None if file doesn't exist or is corrupted (fallback to rebuild)
pub fn try_load_fulltext_index_from_file(
    db_file_path: &str,
    fts_meta: &FulltextIndexMetadata,
) -> Option<FulltextIndex> {
    let ftidx_path = build_fulltext_index_file_path(db_file_path, &fts_meta.name)?;

    if !ftidx_path.exists() {
        return None;
    }

    match FulltextIndex::load_from_file(ftidx_path.clone()) {
        Ok(index) => Some(index),
        Err(e) => {
            tracing::info!(
                path = %ftidx_path.display(),
                error = ?e,
                "Fulltext index cache stale or corrupted, will rebuild from documents"
            );
            None
        }
    }
}

// ============================================================================
// Fuzzy Index Persistence
// ============================================================================

/// Build the .fzidx file path for a fuzzy index
pub fn build_fuzzy_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
    build_index_file_path_with_ext(db_file_path, index_name, "fzidx")
}

/// Try to load a fuzzy index from .fzidx file (graceful degradation)
/// Returns None if file doesn't exist or is corrupted (fallback to rebuild)
pub fn try_load_fuzzy_index_from_file(
    db_file_path: &str,
    fuzzy_meta: &FuzzyIndexMetadata,
) -> Option<FuzzyIndex> {
    let fzidx_path = build_fuzzy_index_file_path(db_file_path, &fuzzy_meta.name)?;

    if !fzidx_path.exists() {
        return None;
    }

    match FuzzyIndex::load_from_file(&fzidx_path) {
        Ok(index) => Some(index),
        Err(e) => {
            tracing::info!(
                path = %fzidx_path.display(),
                error = ?e,
                "Fuzzy index cache stale or corrupted, will rebuild from documents"
            );
            None
        }
    }
}
