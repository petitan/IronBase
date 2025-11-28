// recovery/index_replay.rs
// Parse and replay index changes from WAL

use crate::document::DocumentId;
use crate::error::{MongoLiteError, Result};
use crate::wal::{WALEntry, WALEntryType};

/// Type of index operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexOperation {
    Insert,
    Delete,
}

/// A recovered index change from WAL
#[derive(Debug, Clone)]
pub struct RecoveredIndexChange {
    pub collection: String,
    pub index_name: String,
    pub operation: IndexOperation,
    pub key: serde_json::Value,
    pub doc_id: DocumentId,
}

/// Parses index change entries from WAL
pub struct IndexReplay;

impl IndexReplay {
    /// Parse WAL entries into recovered index changes
    pub fn parse_entries(entries: &[WALEntry]) -> Result<Vec<RecoveredIndexChange>> {
        entries
            .iter()
            .filter(|e| e.entry_type == WALEntryType::IndexChange)
            .map(|e| Self::parse_index_change(&e.data))
            .collect()
    }

    /// Parse a single index change entry
    fn parse_index_change(data: &[u8]) -> Result<RecoveredIndexChange> {
        let json: serde_json::Value = serde_json::from_slice(data)
            .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;

        let collection = json
            .get("collection")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                MongoLiteError::Serialization("Missing collection in index change".into())
            })?
            .to_string();

        let index_name = json
            .get("index_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                MongoLiteError::Serialization("Missing index_name in index change".into())
            })?
            .to_string();

        let operation = match json.get("operation").and_then(|v| v.as_str()) {
            Some("Insert") => IndexOperation::Insert,
            Some("Delete") => IndexOperation::Delete,
            _ => {
                return Err(MongoLiteError::Serialization(
                    "Invalid operation in index change".into(),
                ))
            }
        };

        let key = json
            .get("key")
            .cloned()
            .ok_or_else(|| MongoLiteError::Serialization("Missing key in index change".into()))?;

        let doc_id = Self::parse_doc_id(&json)?;

        Ok(RecoveredIndexChange {
            collection,
            index_name,
            operation,
            key,
            doc_id,
        })
    }

    /// Parse document ID from JSON
    fn parse_doc_id(json: &serde_json::Value) -> Result<DocumentId> {
        let doc_id_value = json.get("doc_id").ok_or_else(|| {
            MongoLiteError::Serialization("Missing doc_id in index change".into())
        })?;

        match doc_id_value {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(DocumentId::Int(i))
                } else {
                    Err(MongoLiteError::Serialization(
                        "Invalid doc_id number type".into(),
                    ))
                }
            }
            serde_json::Value::String(s) => {
                // Check if it looks like an ObjectId
                if s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                    Ok(DocumentId::ObjectId(s.clone()))
                } else {
                    Ok(DocumentId::String(s.clone()))
                }
            }
            serde_json::Value::Object(obj) => {
                // Handle serialized DocumentId enum
                if let Some(i) = obj.get("Int").and_then(|v| v.as_i64()) {
                    Ok(DocumentId::Int(i))
                } else if let Some(s) = obj.get("String").and_then(|v| v.as_str()) {
                    Ok(DocumentId::String(s.to_string()))
                } else if let Some(s) = obj.get("ObjectId").and_then(|v| v.as_str()) {
                    Ok(DocumentId::ObjectId(s.to_string()))
                } else {
                    Err(MongoLiteError::Serialization(
                        "Invalid doc_id object format".into(),
                    ))
                }
            }
            _ => Err(MongoLiteError::Serialization("Invalid doc_id type".into())),
        }
    }
}

/// Statistics from index replay
#[derive(Debug, Default, Clone)]
pub struct IndexReplayStats {
    pub changes_parsed: usize,
    pub inserts: usize,
    pub deletes: usize,
}

impl IndexReplayStats {
    pub fn from_changes(changes: &[RecoveredIndexChange]) -> Self {
        let mut stats = Self::default();
        stats.changes_parsed = changes.len();
        for change in changes {
            match change.operation {
                IndexOperation::Insert => stats.inserts += 1,
                IndexOperation::Delete => stats.deletes += 1,
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_index_change() {
        let data = json!({
            "collection": "users",
            "index_name": "idx_email",
            "operation": "Insert",
            "key": "alice@example.com",
            "doc_id": 42
        });

        let entry_data = serde_json::to_vec(&data).unwrap();
        let entry = WALEntry::new(1, WALEntryType::IndexChange, entry_data);

        let changes = IndexReplay::parse_entries(&[entry]).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].collection, "users");
        assert_eq!(changes[0].index_name, "idx_email");
        assert_eq!(changes[0].operation, IndexOperation::Insert);
        assert!(matches!(changes[0].doc_id, DocumentId::Int(42)));
    }

    #[test]
    fn test_parse_index_change_delete() {
        let data = json!({
            "collection": "orders",
            "index_name": "idx_status",
            "operation": "Delete",
            "key": "pending",
            "doc_id": "order-123"
        });

        let entry_data = serde_json::to_vec(&data).unwrap();
        let entry = WALEntry::new(1, WALEntryType::IndexChange, entry_data);

        let changes = IndexReplay::parse_entries(&[entry]).unwrap();

        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].operation, IndexOperation::Delete);
        assert!(matches!(&changes[0].doc_id, DocumentId::String(s) if s == "order-123"));
    }

    #[test]
    fn test_parse_serialized_doc_id() {
        let data = json!({
            "collection": "test",
            "index_name": "idx",
            "operation": "Insert",
            "key": "value",
            "doc_id": {"Int": 99}
        });

        let entry_data = serde_json::to_vec(&data).unwrap();
        let entry = WALEntry::new(1, WALEntryType::IndexChange, entry_data);

        let changes = IndexReplay::parse_entries(&[entry]).unwrap();
        assert!(matches!(changes[0].doc_id, DocumentId::Int(99)));
    }

    #[test]
    fn test_stats_from_changes() {
        let changes = vec![
            RecoveredIndexChange {
                collection: "a".into(),
                index_name: "idx".into(),
                operation: IndexOperation::Insert,
                key: json!("k1"),
                doc_id: DocumentId::Int(1),
            },
            RecoveredIndexChange {
                collection: "a".into(),
                index_name: "idx".into(),
                operation: IndexOperation::Delete,
                key: json!("k2"),
                doc_id: DocumentId::Int(2),
            },
            RecoveredIndexChange {
                collection: "a".into(),
                index_name: "idx".into(),
                operation: IndexOperation::Insert,
                key: json!("k3"),
                doc_id: DocumentId::Int(3),
            },
        ];

        let stats = IndexReplayStats::from_changes(&changes);
        assert_eq!(stats.changes_parsed, 3);
        assert_eq!(stats.inserts, 2);
        assert_eq!(stats.deletes, 1);
    }
}
