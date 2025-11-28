// recovery/operation_replay.rs
// Replay WAL operations to storage

use crate::document::DocumentId;
use crate::error::{MongoLiteError, Result};
use crate::storage::{RawStorage, Storage};
use crate::transaction::Operation;
use crate::wal::{WALEntry, WALEntryType};

/// Replays WAL operations to storage
///
/// This struct handles the actual application of recovered operations
/// to the storage engine.
pub struct OperationReplay;

impl OperationReplay {
    /// Replay a set of WAL entries to storage
    ///
    /// Only processes Operation entries, ignoring Begin/Commit/Abort markers.
    pub fn replay<S: Storage + RawStorage>(
        storage: &mut S,
        entries: &[WALEntry],
    ) -> Result<ReplayStats> {
        let mut stats = ReplayStats::default();

        for entry in entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::Operation)
        {
            let op: Operation = serde_json::from_slice(&entry.data)
                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;

            Self::apply_operation(storage, &op)?;
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
                let doc_id = Self::extract_doc_id(doc)?;

                // Write document using raw storage
                let doc_json = serde_json::to_string(doc)
                    .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                storage.write_document_raw(collection, &doc_id, doc_json.as_bytes())?;
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
                    .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
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
                    .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
                storage.write_document_raw(collection, doc_id, tombstone_json.as_bytes())?;
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

    /// Extract DocumentId from a document Value
    fn extract_doc_id(doc: &serde_json::Value) -> Result<DocumentId> {
        let id_value = doc
            .get("_id")
            .ok_or_else(|| MongoLiteError::Serialization("Missing _id in document".into()))?;

        match id_value {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(DocumentId::Int(i))
                } else {
                    Err(MongoLiteError::Serialization(
                        "Invalid _id number type".into(),
                    ))
                }
            }
            serde_json::Value::String(s) => {
                // Check if it looks like an ObjectId (24 hex chars)
                if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                    Ok(DocumentId::ObjectId(s.clone()))
                } else {
                    Ok(DocumentId::String(s.clone()))
                }
            }
            _ => Err(MongoLiteError::Serialization(
                "Invalid _id type (must be number or string)".into(),
            )),
        }
    }
}

/// Statistics from operation replay
#[derive(Debug, Default, Clone)]
pub struct ReplayStats {
    pub operations_replayed: usize,
    pub inserts: usize,
    pub updates: usize,
    pub deletes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use serde_json::json;

    #[test]
    fn test_extract_doc_id_int() {
        let doc = json!({"_id": 42, "name": "test"});
        let id = OperationReplay::extract_doc_id(&doc).unwrap();
        assert!(matches!(id, DocumentId::Int(42)));
    }

    #[test]
    fn test_extract_doc_id_string() {
        let doc = json!({"_id": "my-id", "name": "test"});
        let id = OperationReplay::extract_doc_id(&doc).unwrap();
        assert!(matches!(id, DocumentId::String(s) if s == "my-id"));
    }

    #[test]
    fn test_extract_doc_id_objectid() {
        let doc = json!({"_id": "507f1f77bcf86cd799439011", "name": "test"});
        let id = OperationReplay::extract_doc_id(&doc).unwrap();
        assert!(matches!(id, DocumentId::ObjectId(s) if s == "507f1f77bcf86cd799439011"));
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
    }
}
