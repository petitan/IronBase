// recovery/index_replay_ops.rs
// Apply WAL-recovered `Operation`s to loaded non-btree indexes.
//
// Used by `initialize_index_manager` on dirty shutdown to avoid the
// O(N docs) rebuild-from-catalog. Each helper is idempotent: applying
// an op that is already reflected in the index's loaded state is a no-op
// (fulltext and HNSW `insert()` guard on existing id; fuzzy replay
// removes-then-inserts per doc_id at the helper layer).

use serde_json::Value;

use crate::collection_core::doc_id_to_string;
use crate::document::DocumentId;
use crate::error::Result;
use crate::fulltext::FulltextIndex;
use crate::index::fuzzy::FuzzyIndex;
use crate::index::{BPlusTree, IndexKey};
use crate::transaction::Operation;
use crate::value_utils::{get_all_nested_values, get_nested_value};
use crate::vector::HnswIndex;

/// New-state document for an Operation (None for Delete).
fn new_doc(op: &Operation) -> Option<&Value> {
    match op {
        Operation::Insert { doc, .. } => Some(doc),
        Operation::Update { new_doc, .. } => Some(new_doc),
        Operation::Delete { .. } => None,
    }
}

/// Target doc_id of the operation.
fn doc_id_of(op: &Operation) -> &DocumentId {
    match op {
        Operation::Insert { doc_id, .. }
        | Operation::Update { doc_id, .. }
        | Operation::Delete { doc_id, .. } => doc_id,
    }
}

/// Target collection of the operation.
pub(crate) fn collection_of(op: &Operation) -> &str {
    match op {
        Operation::Insert { collection, .. }
        | Operation::Update { collection, .. }
        | Operation::Delete { collection, .. } => collection,
    }
}

/// Apply a single WAL op to a fulltext index on `field`.
///
/// Insert / Update: read `field` as a string. If the index has a
/// `parent_doc_id_field` and the new-state doc supplies a string value for it,
/// call `index.insert_with_parent_doc_id(doc_id, text, parent)` so the
/// `chunk_doc_mapping` stays populated (required by hybrid_search
/// `group_by_document` / document-level AND qualification). Otherwise fall back
/// to plain `index.insert(doc_id, text)`. Both inserts are idempotent thanks
/// to the PR 5 guard.
///
/// If the field is absent or not a string in the new-state doc, any prior
/// entry for `doc_id` is dropped so replay converges to the post-op state.
///
/// Delete: `index.remove(doc_id)`.
pub fn apply_op_to_fulltext(index: &mut FulltextIndex, op: &Operation, field: &str) -> Result<()> {
    let doc_id = doc_id_of(op);
    match op {
        Operation::Insert { .. } | Operation::Update { .. } => {
            if let Some(doc) = new_doc(op) {
                if let Some(Value::String(text)) = get_nested_value(doc, field) {
                    let pdid_field = index.parent_doc_id_field().to_string();
                    let parent_doc_id = get_nested_value(doc, &pdid_field)
                        .and_then(|v| v.as_str().map(|s| s.to_string()));
                    match parent_doc_id {
                        Some(pdid) => index.insert_with_parent_doc_id(doc_id, text, &pdid)?,
                        None => index.insert(doc_id, text)?,
                    }
                    return Ok(());
                }
            }
            index.remove(doc_id)?;
        }
        Operation::Delete { .. } => {
            index.remove(doc_id)?;
        }
    }
    Ok(())
}

/// Apply a single WAL op to a fuzzy index on `field`.
///
/// Fuzzy `insert()` pushes one entry per indexed value (a doc may contribute
/// multiple entries if the field is an array), so a per-insert guard would
/// erase earlier pushes for the same op. Idempotency is therefore enforced
/// at the op level: `remove(doc_id)` wipes any prior state before re-inserting
/// the new-state values.
///
/// Insert / Update: `remove(doc_id)` then one `insert(value, doc_id)` per
/// string value found (handles scalar strings and arrays of strings).
///
/// Delete: `remove(doc_id)`.
pub fn apply_op_to_fuzzy(index: &mut FuzzyIndex, op: &Operation, field: &str) -> Result<()> {
    let doc_id = doc_id_of(op);
    index.remove(doc_id);
    match op {
        Operation::Insert { .. } | Operation::Update { .. } => {
            if let Some(doc) = new_doc(op) {
                if let Some(value) = get_nested_value(doc, field) {
                    match value {
                        Value::String(s) => index.insert(s, doc_id.clone()),
                        Value::Array(items) => {
                            for item in items {
                                if let Value::String(s) = item {
                                    index.insert(s, doc_id.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Operation::Delete { .. } => {}
    }
    Ok(())
}

/// Apply a single WAL op to an HNSW index on `field`.
///
/// Insert / Update: extract `Vec<f32>` from the field (expects a JSON array of
/// numbers). Dimension is validated by `HnswIndex::insert()`; dimension
/// mismatch or an empty/absent vector causes a no-op (we still drop any
/// prior entry so replay converges). The insert is idempotent thanks to
/// the 0260ccda guard (duplicate id → remove-then-insert).
///
/// Delete: `index.remove(&id_str)`.
pub fn apply_op_to_hnsw(index: &mut HnswIndex, op: &Operation, field: &str) -> Result<()> {
    let doc_id = doc_id_of(op);
    let id_str = doc_id_to_string(doc_id);
    match op {
        Operation::Insert { .. } | Operation::Update { .. } => {
            let vector = new_doc(op)
                .and_then(|doc| get_nested_value(doc, field))
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_f64().map(|n| n as f32))
                        .collect::<Vec<f32>>()
                });
            match vector {
                Some(vec) if vec.len() == index.dim() => {
                    index.insert(&id_str, &vec)?;
                }
                _ => {
                    index.remove(&id_str);
                }
            }
        }
        Operation::Delete { .. } => {
            index.remove(&id_str);
        }
    }
    Ok(())
}

/// Apply a single WAL op to a B+ tree index.
///
/// Idempotency: btree `insert(key, doc_id)` is NOT idempotent (unique check
/// errors on duplicate; non-unique would push a second entry). The replay
/// path enforces idempotency at the helper layer with `delete(key, doc_id)`
/// before every `insert(key, doc_id)` — `delete` is a no-op when the entry
/// is absent, so first-time replay works and replay-twice converges to the
/// same single-entry state.
///
/// Insert: extract index keys from `doc` via `tree.extract_keys()` (handles
/// single-field, compound, and multikey/array fields). Apply sparse + null
/// filtering matching `IndexManager::add_document_to_indexes`. For each
/// retained key: `delete(k, doc_id)` then `insert(k, doc_id)`.
///
/// Update: same as Insert for `new_doc`, plus `delete(old_key, doc_id)` for
/// every key extracted from `old_doc` (catches field-value changes).
///
/// Delete: `delete(old_key, doc_id)` for every key from `old_doc`.
///
/// The caller is responsible for marking the index dirty afterwards if the
/// post-replay memory state must be persisted at next checkpoint
/// (`265f0c2e` btree dirty marking rule).
#[allow(dead_code)] // Wired up in Phase B4 (initialize_index_manager replay)
pub fn apply_op_to_btree(tree: &mut BPlusTree, op: &Operation) -> Result<()> {
    let doc_id = doc_id_of(op);

    match op {
        Operation::Insert { doc, .. } => {
            let keys = extract_indexable_btree_keys(tree, doc);
            for k in keys {
                tree.delete(&k, doc_id)?;
                tree.insert(k, doc_id.clone())?;
            }
        }
        Operation::Update {
            old_doc, new_doc, ..
        } => {
            let old_keys = extract_indexable_btree_keys(tree, old_doc);
            let new_keys = extract_indexable_btree_keys(tree, new_doc);
            for k in &old_keys {
                tree.delete(k, doc_id)?;
            }
            for k in new_keys {
                tree.delete(&k, doc_id)?;
                tree.insert(k, doc_id.clone())?;
            }
        }
        Operation::Delete { old_doc, .. } => {
            let keys = extract_indexable_btree_keys(tree, old_doc);
            for k in &keys {
                tree.delete(k, doc_id)?;
            }
        }
    }
    Ok(())
}

/// Mirror of the key-filtering logic from
/// `IndexManager::add_document_to_indexes` (sparse + null + dedup), reused
/// here so replay produces the same set of indexed keys as the runtime path.
#[allow(dead_code)] // Wired up in Phase B4 (initialize_index_manager replay)
fn extract_indexable_btree_keys(tree: &BPlusTree, doc: &Value) -> Vec<IndexKey> {
    // Sparse compound: skip if any indexed field is absent
    if tree.metadata.sparse
        && tree.metadata.is_compound()
        && tree
            .metadata
            .fields
            .iter()
            .any(|field| get_all_nested_values(doc, field).is_empty())
    {
        return Vec::new();
    }

    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for key in tree.extract_keys(doc) {
        if !seen.insert(key.clone()) {
            continue;
        }
        let is_null = crate::index::IndexManager::is_key_all_null(&key);
        let should_index = !is_null || (tree.metadata.unique && !tree.metadata.sparse);
        if should_index {
            result.push(key);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fulltext::{FtsOptions, FulltextIndex};
    use crate::index::fuzzy::{FuzzyAlgorithm, FuzzyIndex};
    use crate::vector::{HnswIndex, VectorIndexConfig};
    use serde_json::json;

    fn doc_id(i: i64) -> DocumentId {
        DocumentId::Int(i)
    }

    fn insert_op(doc_id: DocumentId, doc: Value) -> Operation {
        Operation::Insert {
            collection: "test".to_string(),
            doc_id,
            doc: std::sync::Arc::new(doc),
        }
    }

    fn delete_op(doc_id: DocumentId) -> Operation {
        Operation::Delete {
            collection: "test".to_string(),
            doc_id,
            old_doc: std::sync::Arc::new(json!({})),
        }
    }

    fn update_op(doc_id: DocumentId, new_doc: Value) -> Operation {
        Operation::Update {
            collection: "test".to_string(),
            doc_id,
            old_doc: std::sync::Arc::new(json!({})),
            new_doc: std::sync::Arc::new(new_doc),
        }
    }

    fn make_fts() -> FulltextIndex {
        FulltextIndex::new("fts", "content", FtsOptions::default())
    }

    fn make_hnsw(dim: usize) -> HnswIndex {
        let mut cfg = VectorIndexConfig::default();
        cfg.dim = dim;
        HnswIndex::new(cfg)
    }

    #[test]
    fn fulltext_replay_insert_then_delete() {
        let mut fts = make_fts();
        let op = insert_op(doc_id(1), json!({"content": "hello world"}));
        apply_op_to_fulltext(&mut fts, &op, "content").unwrap();
        assert_eq!(fts.doc_count(), 1);

        apply_op_to_fulltext(&mut fts, &delete_op(doc_id(1)), "content").unwrap();
        assert_eq!(fts.doc_count(), 0);
    }

    #[test]
    fn fulltext_replay_update_replaces_text() {
        let mut fts = make_fts();
        apply_op_to_fulltext(
            &mut fts,
            &insert_op(doc_id(1), json!({"content": "old"})),
            "content",
        )
        .unwrap();
        apply_op_to_fulltext(
            &mut fts,
            &update_op(doc_id(1), json!({"content": "new"})),
            "content",
        )
        .unwrap();
        assert_eq!(fts.doc_count(), 1);
    }

    #[test]
    fn fulltext_replay_is_idempotent() {
        let mut fts = make_fts();
        let op = insert_op(doc_id(1), json!({"content": "hello"}));
        apply_op_to_fulltext(&mut fts, &op, "content").unwrap();
        apply_op_to_fulltext(&mut fts, &op, "content").unwrap();
        apply_op_to_fulltext(&mut fts, &op, "content").unwrap();
        assert_eq!(fts.doc_count(), 1);
    }

    #[test]
    fn fulltext_replay_populates_chunk_doc_mapping_for_rag() {
        // Simulate a RAG-style fulltext index where chunk docs carry a
        // `doc_id` string field pointing at their parent document id.
        let mut fts = FulltextIndex::new("fts", "content", FtsOptions::default());
        // Default parent_doc_id_field is "doc_id" (see FulltextIndex::parent_doc_id_field).

        // Insert three chunks of the same parent document.
        for (cid, text) in &[
            ("c1", "alpha one"),
            ("c2", "alpha two"),
            ("c3", "alpha three"),
        ] {
            let op = insert_op(
                DocumentId::String(cid.to_string()),
                json!({"content": text, "doc_id": "parent-A"}),
            );
            apply_op_to_fulltext(&mut fts, &op, "content").unwrap();
        }

        assert_eq!(fts.doc_count(), 3);
        // Without the chunk mapping, group_by_document-style search breaks
        // silently — the post-replay state MUST mirror the insert-time state.
        assert!(fts.has_chunk_doc_mapping());
        assert_eq!(
            fts.get_chunk_doc_id(&DocumentId::String("c2".to_string())),
            Some("parent-A")
        );
    }

    #[test]
    fn fulltext_replay_missing_field_drops_prior_entry() {
        let mut fts = make_fts();
        apply_op_to_fulltext(
            &mut fts,
            &insert_op(doc_id(1), json!({"content": "hello"})),
            "content",
        )
        .unwrap();
        apply_op_to_fulltext(
            &mut fts,
            &update_op(doc_id(1), json!({"other": "field"})),
            "content",
        )
        .unwrap();
        assert_eq!(fts.doc_count(), 0);
    }

    #[test]
    fn fuzzy_replay_insert_then_delete() {
        let mut fz = FuzzyIndex::new("fz", "name", FuzzyAlgorithm::JaroWinkler, 0.8);
        let op = insert_op(doc_id(1), json!({"name": "Alice"}));
        apply_op_to_fuzzy(&mut fz, &op, "name").unwrap();
        assert_eq!(fz.entry_count(), 1);
        apply_op_to_fuzzy(&mut fz, &delete_op(doc_id(1)), "name").unwrap();
        assert_eq!(fz.entry_count(), 0);
    }

    #[test]
    fn fuzzy_replay_update_replaces_all_values() {
        let mut fz = FuzzyIndex::new("fz", "tags", FuzzyAlgorithm::JaroWinkler, 0.8);
        apply_op_to_fuzzy(
            &mut fz,
            &insert_op(doc_id(1), json!({"tags": ["a", "b", "c"]})),
            "tags",
        )
        .unwrap();
        assert_eq!(fz.entry_count(), 3);
        apply_op_to_fuzzy(
            &mut fz,
            &update_op(doc_id(1), json!({"tags": ["x"]})),
            "tags",
        )
        .unwrap();
        assert_eq!(fz.entry_count(), 1);
    }

    #[test]
    fn fuzzy_replay_is_idempotent() {
        let mut fz = FuzzyIndex::new("fz", "name", FuzzyAlgorithm::JaroWinkler, 0.8);
        let op = insert_op(doc_id(1), json!({"name": "Alice"}));
        apply_op_to_fuzzy(&mut fz, &op, "name").unwrap();
        apply_op_to_fuzzy(&mut fz, &op, "name").unwrap();
        apply_op_to_fuzzy(&mut fz, &op, "name").unwrap();
        assert_eq!(fz.entry_count(), 1);
    }

    #[test]
    fn hnsw_replay_insert_then_delete() {
        let mut hnsw = make_hnsw(3);
        let op = insert_op(doc_id(1), json!({"v": [1.0, 2.0, 3.0]}));
        apply_op_to_hnsw(&mut hnsw, &op, "v").unwrap();
        assert_eq!(hnsw.len(), 1);
        apply_op_to_hnsw(&mut hnsw, &delete_op(doc_id(1)), "v").unwrap();
        assert_eq!(hnsw.len(), 0);
    }

    #[test]
    fn hnsw_replay_dim_mismatch_drops_prior() {
        let mut hnsw = make_hnsw(3);
        apply_op_to_hnsw(
            &mut hnsw,
            &insert_op(doc_id(1), json!({"v": [1.0, 2.0, 3.0]})),
            "v",
        )
        .unwrap();
        apply_op_to_hnsw(
            &mut hnsw,
            &update_op(doc_id(1), json!({"v": [1.0, 2.0]})),
            "v",
        )
        .unwrap();
        assert_eq!(hnsw.len(), 0);
    }

    #[test]
    fn hnsw_replay_is_idempotent() {
        let mut hnsw = make_hnsw(3);
        let op = insert_op(doc_id(1), json!({"v": [1.0, 2.0, 3.0]}));
        apply_op_to_hnsw(&mut hnsw, &op, "v").unwrap();
        apply_op_to_hnsw(&mut hnsw, &op, "v").unwrap();
        apply_op_to_hnsw(&mut hnsw, &op, "v").unwrap();
        assert_eq!(hnsw.len(), 1);
    }

    #[test]
    fn collection_of_returns_correct_name() {
        let op = insert_op(doc_id(1), json!({"x": 1}));
        assert_eq!(collection_of(&op), "test");
    }
}
