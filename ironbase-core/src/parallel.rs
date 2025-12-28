// ironbase-core/src/parallel.rs
//! Parallel iteration utilities using Rayon
//!
//! This module is only compiled when the `parallel` feature is enabled.
//! It provides parallel versions of common collection operations.

use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

use crate::document::DocumentId;
use crate::index::IndexKey;
use crate::value_utils::get_nested_value;

/// Parallel JSON deserialization for document batches
///
/// Takes raw bytes and deserializes them in parallel across CPU cores.
/// Memory efficient: uses into_par_iter() to consume input, avoiding copies.
///
/// NOTE: Currently unused after streaming refactor (was used by scan_documents_via_catalog).
#[allow(dead_code)]
pub fn parallel_deserialize(raw_docs: Vec<(DocumentId, Vec<u8>)>) -> HashMap<DocumentId, Value> {
    raw_docs
        .into_par_iter()
        .filter_map(|(doc_id, bytes)| {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .map(|doc| (doc_id, doc))
        })
        .collect()
}

/// Parallel JSON deserialization returning Vec (preserves order better)
#[allow(dead_code)]
pub fn parallel_deserialize_vec(raw_docs: Vec<(DocumentId, Vec<u8>)>) -> Vec<(DocumentId, Value)> {
    raw_docs
        .into_par_iter()
        .filter_map(|(doc_id, bytes)| {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .map(|doc| (doc_id, doc))
        })
        .collect()
}

/// Parallel filter with predicate
///
/// Filters items in parallel. Use only when predicate is CPU-intensive
/// and there's no early termination (limit).
#[allow(dead_code)]
pub fn parallel_filter<T, F>(items: Vec<T>, predicate: F) -> Vec<T>
where
    T: Send + Sync,
    F: Fn(&T) -> bool + Send + Sync,
{
    items.into_par_iter().filter(predicate).collect()
}

/// Parallel map operation
#[allow(dead_code)]
pub fn parallel_map<T, U, F>(items: Vec<T>, mapper: F) -> Vec<U>
where
    T: Send,
    U: Send,
    F: Fn(T) -> U + Send + Sync,
{
    items.into_par_iter().map(mapper).collect()
}

/// Parallel filter_map operation
#[allow(dead_code)]
pub fn parallel_filter_map<T, U, F>(items: Vec<T>, f: F) -> Vec<U>
where
    T: Send,
    U: Send,
    F: Fn(T) -> Option<U> + Send + Sync,
{
    items.into_par_iter().filter_map(f).collect()
}

/// Parallel index key extraction for single-field indexes
///
/// Extracts keys from documents in parallel using the specified field.
/// Documents with missing fields are SKIPPED (matches original filter_map behavior).
/// Memory efficient: uses into_par_iter() to consume input Vec.
#[allow(dead_code)]
pub fn parallel_extract_keys_single(
    docs: Vec<(DocumentId, Value)>,
    field: &str,
) -> Vec<(IndexKey, DocumentId)> {
    docs.into_par_iter()
        .filter_map(|(doc_id, doc)| {
            get_nested_value(&doc, field).map(|field_value| (IndexKey::from(field_value), doc_id))
        })
        .collect()
}

/// Parallel index key extraction for compound indexes
///
/// Extracts compound keys from documents in parallel using multiple fields.
/// Memory efficient: uses into_par_iter() to consume input Vec.
#[allow(dead_code)]
pub fn parallel_extract_keys_compound(
    docs: Vec<(DocumentId, Value)>,
    fields: &[String],
) -> Vec<(IndexKey, DocumentId)> {
    docs.into_par_iter()
        .map(|(doc_id, doc)| {
            let keys: Vec<IndexKey> = fields
                .iter()
                .map(|field| {
                    get_nested_value(&doc, field)
                        .map(IndexKey::from)
                        .unwrap_or(IndexKey::Null)
                })
                .collect();
            (IndexKey::Compound(keys), doc_id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parallel_deserialize() {
        let raw_docs: Vec<(DocumentId, Vec<u8>)> = (0..100)
            .map(|i| {
                let doc = json!({"_id": i, "name": format!("doc_{}", i)});
                (DocumentId::Int(i), serde_json::to_vec(&doc).unwrap())
            })
            .collect();

        let result = parallel_deserialize(raw_docs);
        assert_eq!(result.len(), 100);
        assert!(result.contains_key(&DocumentId::Int(0)));
        assert!(result.contains_key(&DocumentId::Int(99)));
    }

    #[test]
    fn test_parallel_filter() {
        let items: Vec<i32> = (0..1000).collect();
        let evens = parallel_filter(items, |x| x % 2 == 0);
        assert_eq!(evens.len(), 500);
    }

    #[test]
    fn test_parallel_map() {
        let items: Vec<i32> = (0..100).collect();
        let doubled = parallel_map(items, |x| x * 2);
        assert_eq!(doubled.len(), 100);
        assert_eq!(doubled[0], 0);
        assert_eq!(doubled[50], 100);
    }
}
