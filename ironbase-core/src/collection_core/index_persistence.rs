use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::fulltext::{FulltextIndex, FulltextIndexMetadata};
use crate::index::fuzzy::{FuzzyIndex, FuzzyIndexMetadata};
use crate::index::{BPlusTree, IndexMetadata};

fn sanitize_component(name: &str) -> String {
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

fn build_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
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

    let file_name = format!("{}_{}_{:08x}.idx", stem, safe_component, hash as u32);
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(parent.join(file_name))
}

pub fn persist_index_to_disk<F, T>(db_file_path: &str, index_name: &str, save_fn: F) -> Result<()>
where
    F: FnOnce(&mut File) -> Result<T>,
{
    if let Some(index_file_path) = build_index_file_path(db_file_path, index_name) {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&index_file_path)?;
        save_fn(&mut file)?;
    }
    Ok(())
}

/// Try to load an index from .idx file (graceful degradation)
/// Returns None if file doesn't exist or is corrupted (fallback to rebuild)
pub fn try_load_index_from_file(
    db_file_path: &str,
    index_meta: &IndexMetadata,
) -> Option<BPlusTree> {
    let idx_path = build_index_file_path(db_file_path, &index_meta.name)?;

    if !idx_path.exists() {
        return None;
    }

    let mut file = File::open(&idx_path).ok()?;
    BPlusTree::load_from_file(&mut file, index_meta.clone()).ok()
}

// ============================================================================
// Fulltext Index Persistence
// ============================================================================

/// Build the .ftidx file path for a fulltext index
pub fn build_fulltext_index_file_path(db_file_path: &str, index_name: &str) -> Option<PathBuf> {
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

    let file_name = format!("{}_{}_{:08x}.ftidx", stem, safe_component, hash as u32);
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(parent.join(file_name))
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
            eprintln!(
                "[WARN] Failed to load fulltext index from {:?}: {:?}",
                ftidx_path, e
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

    let file_name = format!("{}_{}_{:08x}.fzidx", stem, safe_component, hash as u32);
    let parent = base_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Some(parent.join(file_name))
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
            eprintln!(
                "[WARN] Failed to load fuzzy index from {:?}: {:?}",
                fzidx_path, e
            );
            None
        }
    }
}
