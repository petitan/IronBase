// recovery/operation_replay.rs
// Replay WAL operations to storage

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::recovery::RecoveryMode;
use crate::storage::{RawStorage, Storage};
use crate::transaction::Operation;
use crate::wal::{WALEntry, WALEntryType};

/// Replays WAL operations to storage
///
/// This struct handles the actual application of recovered operations
/// to the storage engine.
pub struct OperationReplay;

/// Truncate bytes to a UTF-8 safe string preview for diagnostics.
fn data_preview(data: &[u8]) -> String {
    const MAX: usize = 200;
    let cut = data.len().min(MAX);
    String::from_utf8_lossy(&data[..cut]).into_owned()
}

impl OperationReplay {
    /// Replay a set of WAL entries to storage.
    ///
    /// Only processes Operation entries, ignoring Begin/Commit/Abort markers.
    /// The recovery mode is read from the `IRONBASE_RECOVERY_MODE` env var
    /// (`strict` by default, `best_effort` to skip bad entries on corruption).
    pub fn replay<S: Storage + RawStorage>(
        storage: &mut S,
        entries: &[WALEntry],
    ) -> Result<ReplayStats> {
        Self::replay_with_mode(storage, entries, RecoveryMode::from_env())
    }

    /// Replay WAL entries with an explicit recovery mode.
    ///
    /// In `Strict` mode, any deserialize or apply failure aborts recovery.
    /// In `BestEffort` mode, failures are logged at WARN and the offending
    /// entry is skipped (`ReplayStats::entries_skipped` is incremented);
    /// this may cause data loss and is intended for emergency recovery.
    pub fn replay_with_mode<S: Storage + RawStorage>(
        storage: &mut S,
        entries: &[WALEntry],
        mode: RecoveryMode,
    ) -> Result<ReplayStats> {
        let mut stats = ReplayStats::default();

        for entry in entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::Operation)
        {
            let op: Operation = match serde_json::from_slice(&entry.data) {
                Ok(op) => op,
                Err(e) => match mode {
                    RecoveryMode::Strict => {
                        tracing::error!(
                            tx_id = entry.transaction_id,
                            entry_size = entry.data.len(),
                            data_preview = %data_preview(&entry.data),
                            error = %e,
                            "WAL replay: Operation deserialization failed (possible schema mismatch or corruption)"
                        );
                        return Err(IronBaseError::Serialization(format!(
                            "WAL replay failed at tx_id={}: {}. Set IRONBASE_RECOVERY_MODE=best_effort to skip bad entries (may cause data loss).",
                            entry.transaction_id, e
                        )));
                    }
                    RecoveryMode::BestEffort => {
                        tracing::warn!(
                            tx_id = entry.transaction_id,
                            entry_size = entry.data.len(),
                            data_preview = %data_preview(&entry.data),
                            error = %e,
                            "WAL replay: SKIPPING bad Operation entry (best_effort mode — DATA LOSS POSSIBLE)"
                        );
                        stats.entries_skipped += 1;
                        continue;
                    }
                },
            };

            if let Err(e) = Self::apply_operation(storage, &op) {
                match mode {
                    RecoveryMode::Strict => return Err(e),
                    RecoveryMode::BestEffort => {
                        tracing::warn!(
                            tx_id = entry.transaction_id,
                            error = %e,
                            "WAL replay: SKIPPING Operation — apply_operation failed (best_effort mode — DATA LOSS POSSIBLE)"
                        );
                        stats.entries_skipped += 1;
                        continue;
                    }
                }
            }

            stats.operations_replayed += 1;

            match op {
                Operation::Insert { .. } => stats.inserts += 1,
                Operation::Update { .. } => stats.updates += 1,
                Operation::Delete { .. } => stats.deletes += 1,
            }
        }

        Ok(stats)
    }

    /// Apply a single operation to storage
    fn apply_operation<S: Storage + RawStorage>(storage: &mut S, op: &Operation) -> Result<()> {
        match op {
            Operation::Insert {
                collection, doc, ..
            } => {
                // Ensure collection exists
                let _ = storage.create_collection(collection);

                // Extract document ID
                let doc_id = DocumentId::extract_from_value(doc)?;

                // Write document using raw storage
                let doc_json = serde_json::to_string(doc)
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                storage.write_document_raw(collection, &doc_id, doc_json.as_bytes())?;

                // Update live document count for correct count_documents({}) after recovery
                storage.adjust_live_count(collection, 1);
            }

            Operation::Update {
                collection,
                doc_id,
                new_doc,
                ..
            } => {
                // Ensure collection exists
                let _ = storage.create_collection(collection);

                // Write updated document using raw storage
                let doc_json = serde_json::to_string(new_doc)
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                storage.write_document_raw(collection, doc_id, doc_json.as_bytes())?;
            }

            Operation::Delete {
                collection, doc_id, ..
            } => {
                // Ensure collection exists
                let _ = storage.create_collection(collection);

                // Write tombstone marker
                let tombstone = serde_json::json!({
                    "_id": Self::doc_id_to_value(doc_id),
                    "_collection": collection,
                    "_tombstone": true
                });
                let tombstone_json = serde_json::to_string(&tombstone)
                    .map_err(|e| IronBaseError::Serialization(e.to_string()))?;
                storage.write_document_raw(collection, doc_id, tombstone_json.as_bytes())?;

                // Update live document count for correct count_documents({}) after recovery
                storage.adjust_live_count(collection, -1);
            }
        }

        Ok(())
    }

    /// Convert DocumentId to serde_json::Value
    fn doc_id_to_value(doc_id: &DocumentId) -> serde_json::Value {
        match doc_id {
            DocumentId::Int(i) => serde_json::json!(i),
            DocumentId::String(s) => serde_json::json!(s),
            DocumentId::ObjectId(s) => serde_json::json!(s),
        }
    }

    // NOTE: Use DocumentId::extract_from_value() for _id extraction
}

/// Statistics from operation replay
#[derive(Debug, Default, Clone)]
pub struct ReplayStats {
    pub operations_replayed: usize,
    pub inserts: usize,
    pub updates: usize,
    pub deletes: usize,
    /// Number of entries skipped due to deserialize/apply errors (best_effort mode only)
    pub entries_skipped: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use serde_json::json;

    #[test]
    fn test_extract_doc_id_int() {
        let doc = json!({"_id": 42, "name": "test"});
        let id = DocumentId::extract_from_value(&doc).unwrap();
        assert!(matches!(id, DocumentId::Int(42)));
    }

    #[test]
    fn test_extract_doc_id_string() {
        let doc = json!({"_id": "my-id", "name": "test"});
        let id = DocumentId::extract_from_value(&doc).unwrap();
        assert!(matches!(id, DocumentId::String(s) if s == "my-id"));
    }

    #[test]
    fn test_extract_doc_id_objectid() {
        let doc = json!({"_id": "507f1f77bcf86cd799439011", "name": "test"});
        let id = DocumentId::extract_from_value(&doc).unwrap();
        assert!(matches!(id, DocumentId::ObjectId(s) if s == "507f1f77bcf86cd799439011"));
    }

    #[test]
    fn test_extract_doc_id_missing() {
        let doc = json!({"name": "test"});
        let result = DocumentId::extract_from_value(&doc);
        assert!(result.is_err());
    }

    #[test]
    fn test_doc_id_to_value_int() {
        let doc_id = DocumentId::Int(42);
        let value = OperationReplay::doc_id_to_value(&doc_id);
        assert_eq!(value, json!(42));
    }

    #[test]
    fn test_doc_id_to_value_string() {
        let doc_id = DocumentId::String("my-id".to_string());
        let value = OperationReplay::doc_id_to_value(&doc_id);
        assert_eq!(value, json!("my-id"));
    }

    #[test]
    fn test_doc_id_to_value_objectid() {
        let doc_id = DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string());
        let value = OperationReplay::doc_id_to_value(&doc_id);
        assert_eq!(value, json!("507f1f77bcf86cd799439011"));
    }

    #[test]
    fn test_replay_insert() {
        let mut storage = MemoryStorage::new();

        let op = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"_id": 1, "name": "Alice"}),
        };

        let entry_data = serde_json::to_vec(&op).unwrap();
        let entry = WALEntry::new(1, WALEntryType::Operation, entry_data);

        let stats = OperationReplay::replay(&mut storage, &[entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);
        assert_eq!(stats.updates, 0);
        assert_eq!(stats.deletes, 0);
    }

    #[test]
    fn test_replay_update() {
        let mut storage = MemoryStorage::new();

        // First insert a document
        let insert_op = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"_id": 1, "name": "Alice"}),
        };

        let insert_entry_data = serde_json::to_vec(&insert_op).unwrap();
        let insert_entry = WALEntry::new(1, WALEntryType::Operation, insert_entry_data);
        OperationReplay::replay(&mut storage, &[insert_entry]).unwrap();

        // Now update the document
        let update_op = Operation::Update {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: json!({"_id": 1, "name": "Alice"}),
            new_doc: json!({"_id": 1, "name": "Alice Updated"}),
        };

        let update_entry_data = serde_json::to_vec(&update_op).unwrap();
        let update_entry = WALEntry::new(2, WALEntryType::Operation, update_entry_data);

        let stats = OperationReplay::replay(&mut storage, &[update_entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.updates, 1);
        assert_eq!(stats.inserts, 0);
        assert_eq!(stats.deletes, 0);
    }

    #[test]
    fn test_replay_delete() {
        let mut storage = MemoryStorage::new();

        // First insert a document
        let insert_op = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"_id": 1, "name": "Alice"}),
        };

        let insert_entry_data = serde_json::to_vec(&insert_op).unwrap();
        let insert_entry = WALEntry::new(1, WALEntryType::Operation, insert_entry_data);
        OperationReplay::replay(&mut storage, &[insert_entry]).unwrap();

        // Now delete the document
        let delete_op = Operation::Delete {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            old_doc: json!({"_id": 1, "name": "Alice"}),
        };

        let delete_entry_data = serde_json::to_vec(&delete_op).unwrap();
        let delete_entry = WALEntry::new(2, WALEntryType::Operation, delete_entry_data);

        let stats = OperationReplay::replay(&mut storage, &[delete_entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.deletes, 1);
        assert_eq!(stats.inserts, 0);
        assert_eq!(stats.updates, 0);
    }

    #[test]
    fn test_replay_multiple_operations() {
        let mut storage = MemoryStorage::new();

        let ops = [
            Operation::Insert {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(1),
                doc: json!({"_id": 1, "name": "Alice"}),
            },
            Operation::Insert {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(2),
                doc: json!({"_id": 2, "name": "Bob"}),
            },
            Operation::Update {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(1),
                old_doc: json!({"_id": 1, "name": "Alice"}),
                new_doc: json!({"_id": 1, "name": "Alice Smith"}),
            },
            Operation::Delete {
                collection: "users".to_string(),
                doc_id: DocumentId::Int(2),
                old_doc: json!({"_id": 2, "name": "Bob"}),
            },
        ];

        let entries: Vec<WALEntry> = ops
            .iter()
            .enumerate()
            .map(|(i, op)| {
                let data = serde_json::to_vec(op).unwrap();
                WALEntry::new((i + 1) as u64, WALEntryType::Operation, data)
            })
            .collect();

        let stats = OperationReplay::replay(&mut storage, &entries).unwrap();

        assert_eq!(stats.operations_replayed, 4);
        assert_eq!(stats.inserts, 2);
        assert_eq!(stats.updates, 1);
        assert_eq!(stats.deletes, 1);
    }

    #[test]
    fn test_replay_ignores_non_operation_entries() {
        let mut storage = MemoryStorage::new();

        let entries = vec![
            WALEntry::new(1, WALEntryType::Begin, vec![]),
            WALEntry::new(
                1,
                WALEntryType::Operation,
                serde_json::to_vec(&Operation::Insert {
                    collection: "test".to_string(),
                    doc_id: DocumentId::Int(1),
                    doc: json!({"_id": 1, "name": "Alice"}),
                })
                .unwrap(),
            ),
            WALEntry::new(1, WALEntryType::Commit, vec![]),
        ];

        let stats = OperationReplay::replay(&mut storage, &entries).unwrap();

        // Should only count the Operation entry, not Begin/Commit
        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);
    }

    #[test]
    fn test_replay_empty_entries() {
        let mut storage = MemoryStorage::new();
        let entries: Vec<WALEntry> = vec![];

        let stats = OperationReplay::replay(&mut storage, &entries).unwrap();

        assert_eq!(stats.operations_replayed, 0);
        assert_eq!(stats.inserts, 0);
        assert_eq!(stats.updates, 0);
        assert_eq!(stats.deletes, 0);
    }

    #[test]
    fn test_replay_creates_collection_if_not_exists() {
        let mut storage = MemoryStorage::new();

        // Insert into a collection that doesn't exist
        let op = Operation::Insert {
            collection: "new_collection".to_string(),
            doc_id: DocumentId::String("doc1".to_string()),
            doc: json!({"_id": "doc1", "data": "test"}),
        };

        let entry_data = serde_json::to_vec(&op).unwrap();
        let entry = WALEntry::new(1, WALEntryType::Operation, entry_data);

        let stats = OperationReplay::replay(&mut storage, &[entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);

        // Verify collection was created by checking it's in the list
        let collections = storage.list_collections();
        assert!(collections.contains(&"new_collection".to_string()));
    }

    #[test]
    fn test_replay_stats_default() {
        let stats = ReplayStats::default();
        assert_eq!(stats.operations_replayed, 0);
        assert_eq!(stats.inserts, 0);
        assert_eq!(stats.updates, 0);
        assert_eq!(stats.deletes, 0);
    }

    #[test]
    fn test_replay_with_string_doc_id() {
        let mut storage = MemoryStorage::new();

        let op = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::String("my-custom-id".to_string()),
            doc: json!({"_id": "my-custom-id", "name": "Test"}),
        };

        let entry_data = serde_json::to_vec(&op).unwrap();
        let entry = WALEntry::new(1, WALEntryType::Operation, entry_data);

        let stats = OperationReplay::replay(&mut storage, &[entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);
    }

    #[test]
    fn test_replay_with_objectid_doc_id() {
        let mut storage = MemoryStorage::new();

        let op = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string()),
            doc: json!({"_id": "507f1f77bcf86cd799439011", "name": "Test"}),
        };

        let entry_data = serde_json::to_vec(&op).unwrap();
        let entry = WALEntry::new(1, WALEntryType::Operation, entry_data);

        let stats = OperationReplay::replay(&mut storage, &[entry]).unwrap();

        assert_eq!(stats.operations_replayed, 1);
        assert_eq!(stats.inserts, 1);
    }

    fn build_mixed_entries() -> Vec<WALEntry> {
        let good1 = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"_id": 1, "name": "Alice"}),
        };
        let good2 = Operation::Insert {
            collection: "test".to_string(),
            doc_id: DocumentId::Int(3),
            doc: json!({"_id": 3, "name": "Charlie"}),
        };
        vec![
            WALEntry::new(
                1,
                WALEntryType::Operation,
                serde_json::to_vec(&good1).unwrap(),
            ),
            WALEntry::new(
                2,
                WALEntryType::Operation,
                b"not valid json at all".to_vec(),
            ),
            WALEntry::new(
                3,
                WALEntryType::Operation,
                serde_json::to_vec(&good2).unwrap(),
            ),
        ]
    }

    #[test]
    fn test_replay_strict_mode_fails_on_corrupt_entry() {
        let mut storage = MemoryStorage::new();
        let entries = build_mixed_entries();

        let result =
            OperationReplay::replay_with_mode(&mut storage, &entries, RecoveryMode::Strict);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tx_id=2") && err.contains("IRONBASE_RECOVERY_MODE=best_effort"),
            "error must mention tx_id and the best_effort hint, got: {err}"
        );
    }

    #[test]
    fn test_replay_best_effort_skips_corrupt_entry() {
        let mut storage = MemoryStorage::new();
        let entries = build_mixed_entries();

        let stats =
            OperationReplay::replay_with_mode(&mut storage, &entries, RecoveryMode::BestEffort)
                .expect("best_effort must not propagate deserialize errors");

        assert_eq!(stats.operations_replayed, 2);
        assert_eq!(stats.inserts, 2);
        assert_eq!(stats.entries_skipped, 1);
    }
}
