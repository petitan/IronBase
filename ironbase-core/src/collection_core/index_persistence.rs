use std::collections::hash_map::DefaultHasher;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::Result;

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
