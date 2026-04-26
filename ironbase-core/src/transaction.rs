// ironbase-core/src/transaction.rs
// Transaction management for ACID (Atomicity, Consistency, Isolation, Durability)
// Isolation level: Read Committed (exclusive write lock, SQLite-style)

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};

fn serialize_arc_value<S: Serializer>(
    arc: &Arc<Value>,
    ser: S,
) -> std::result::Result<S::Ok, S::Error> {
    (**arc).serialize(ser)
}

fn deserialize_arc_value<'de, D: Deserializer<'de>>(
    de: D,
) -> std::result::Result<Arc<Value>, D::Error> {
    Value::deserialize(de).map(Arc::new)
}

/// Unique transaction identifier
pub type TransactionId = u64;

/// Transaction state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionState {
    /// Transaction is active and accepting operations
    Active,
    /// Transaction has been successfully committed
    Committed,
    /// Transaction has been rolled back
    Aborted,
}

/// A single operation within a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    /// Insert a new document
    Insert {
        collection: String,
        doc_id: DocumentId,
        #[serde(
            serialize_with = "serialize_arc_value",
            deserialize_with = "deserialize_arc_value"
        )]
        doc: Arc<Value>,
    },
    /// Update an existing document
    Update {
        collection: String,
        doc_id: DocumentId,
        #[serde(
            serialize_with = "serialize_arc_value",
            deserialize_with = "deserialize_arc_value"
        )]
        old_doc: Arc<Value>,
        #[serde(
            serialize_with = "serialize_arc_value",
            deserialize_with = "deserialize_arc_value"
        )]
        new_doc: Arc<Value>,
    },
    /// Delete a document
    Delete {
        collection: String,
        doc_id: DocumentId,
        #[serde(
            serialize_with = "serialize_arc_value",
            deserialize_with = "deserialize_arc_value"
        )]
        old_doc: Arc<Value>, // For potential rollback
    },
}

/// Index change to be applied atomically
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexChange {
    pub operation: IndexOperation,
    pub key: crate::index::IndexKey,
    pub doc_id: DocumentId,
}

/// Index operation type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexOperation {
    Insert,
    Delete,
}

/// Collection metadata changes (e.g., last_id increments)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataChange {
    pub collection: String,
    pub last_id: i64,
}

/// A transaction groups multiple operations for atomic execution
#[derive(Debug, Clone)]
pub struct Transaction {
    /// Unique transaction ID
    pub id: TransactionId,

    /// List of buffered operations
    operations: Vec<Operation>,

    /// Index changes to apply atomically
    index_changes: HashMap<String, Vec<IndexChange>>,

    /// Metadata changes (last_id, etc.)
    metadata_changes: Vec<MetadataChange>,

    /// Current state
    state: TransactionState,

    /// Flag indicating operations were already applied (e.g., auto-commit fast path)
    operations_applied: bool,

    /// Whether this transaction holds the exclusive write lock
    has_write_lock: bool,

    /// When the write lock was acquired (for diagnostics/timeout)
    write_lock_acquired_at: Option<std::time::Instant>,
}

impl Transaction {
    /// Create a new active transaction
    pub fn new(id: TransactionId) -> Self {
        Transaction {
            id,
            operations: Vec::new(),
            index_changes: HashMap::new(),
            metadata_changes: Vec::new(),
            state: TransactionState::Active,
            operations_applied: false,
            has_write_lock: false,
            write_lock_acquired_at: None,
        }
    }

    /// Get current state
    pub fn state(&self) -> TransactionState {
        self.state
    }

    /// Check if transaction is active
    pub fn is_active(&self) -> bool {
        self.state == TransactionState::Active
    }

    /// Add an operation to the transaction buffer
    pub fn add_operation(&mut self, op: Operation) -> Result<()> {
        if !self.is_active() {
            return Err(IronBaseError::TransactionCommitted);
        }
        self.operations.push(op);
        Ok(())
    }

    /// Add an index change to be applied on commit
    pub fn add_index_change(&mut self, index_name: String, change: IndexChange) -> Result<()> {
        if !self.is_active() {
            return Err(IronBaseError::TransactionCommitted);
        }
        self.index_changes
            .entry(index_name)
            .or_default()
            .push(change);
        Ok(())
    }

    /// Add a metadata change
    pub fn add_metadata_change(&mut self, change: MetadataChange) -> Result<()> {
        if !self.is_active() {
            return Err(IronBaseError::TransactionCommitted);
        }
        self.metadata_changes.push(change);
        Ok(())
    }

    /// Mark that operations have already been applied to storage/indexes
    pub fn mark_operations_applied(&mut self) {
        self.operations_applied = true;
    }

    /// Check whether operations were applied outside the WAL commit
    pub fn operations_applied(&self) -> bool {
        self.operations_applied
    }

    /// Get all operations (for WAL writing)
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Get all index changes
    pub fn index_changes(&self) -> &HashMap<String, Vec<IndexChange>> {
        &self.index_changes
    }

    /// Get all metadata changes
    pub fn metadata_changes(&self) -> &[MetadataChange] {
        &self.metadata_changes
    }

    /// Mark transaction as committed
    pub fn mark_committed(&mut self) -> Result<()> {
        if !self.is_active() {
            return Err(IronBaseError::TransactionCommitted);
        }
        self.state = TransactionState::Committed;
        Ok(())
    }

    /// Rollback transaction (discard all buffered operations)
    pub fn rollback(&mut self) -> Result<()> {
        self.operations.clear();
        self.index_changes.clear();
        self.metadata_changes.clear();
        self.state = TransactionState::Aborted;
        Ok(())
    }

    /// Get number of operations in transaction
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    /// Mark that this transaction has acquired the exclusive write lock
    pub fn mark_write_lock_acquired(&mut self) {
        self.has_write_lock = true;
        self.write_lock_acquired_at = Some(std::time::Instant::now());
    }

    /// Check if this transaction holds the write lock
    pub fn has_write_lock(&self) -> bool {
        self.has_write_lock
    }

    /// Get how long the write lock has been held (for diagnostics)
    pub fn write_lock_duration(&self) -> Option<std::time::Duration> {
        self.write_lock_acquired_at.map(|t| t.elapsed())
    }

    /// Clear the write lock flag (called on commit/rollback)
    pub fn clear_write_lock(&mut self) {
        self.has_write_lock = false;
        self.write_lock_acquired_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexKey;
    use serde_json::json;

    #[test]
    fn test_transaction_new() {
        let tx = Transaction::new(1);
        assert_eq!(tx.id, 1);
        assert_eq!(tx.state(), TransactionState::Active);
        assert!(tx.is_active());
        assert_eq!(tx.operation_count(), 0);
    }

    #[test]
    fn test_add_operation_when_active() {
        let mut tx = Transaction::new(1);

        let op = Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: Arc::new(json!({"name": "Alice"})),
        };

        assert!(tx.add_operation(op).is_ok());
        assert_eq!(tx.operation_count(), 1);
    }

    #[test]
    fn test_add_operation_when_committed() {
        let mut tx = Transaction::new(1);
        tx.mark_committed().unwrap();

        let op = Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: Arc::new(json!({"name": "Alice"})),
        };

        assert!(matches!(
            tx.add_operation(op),
            Err(IronBaseError::TransactionCommitted)
        ));
    }

    #[test]
    fn test_rollback() {
        let mut tx = Transaction::new(1);

        let op = Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: Arc::new(json!({"name": "Alice"})),
        };
        tx.add_operation(op).unwrap();

        assert_eq!(tx.operation_count(), 1);

        tx.rollback().unwrap();

        assert_eq!(tx.state(), TransactionState::Aborted);
        assert_eq!(tx.operation_count(), 0);
    }

    #[test]
    fn test_index_key_from_value() {
        assert_eq!(IndexKey::from(&json!(42)), IndexKey::Int(42));
        assert_eq!(
            IndexKey::from(&json!("test")),
            IndexKey::String("test".to_string())
        );
        assert_eq!(IndexKey::from(&json!(true)), IndexKey::Bool(true));
        assert_eq!(IndexKey::from(&json!(null)), IndexKey::Null);
    }

    #[test]
    fn test_add_index_change() {
        let mut tx = Transaction::new(1);

        let change = IndexChange {
            operation: IndexOperation::Insert,
            key: IndexKey::Int(1),
            doc_id: DocumentId::Int(1),
        };

        tx.add_index_change("users_id".to_string(), change).unwrap();

        assert_eq!(tx.index_changes().len(), 1);
        assert!(tx.index_changes().contains_key("users_id"));
    }

    #[test]
    fn test_add_metadata_change() {
        let mut tx = Transaction::new(1);

        let change = MetadataChange {
            collection: "users".to_string(),
            last_id: 10,
        };

        tx.add_metadata_change(change).unwrap();

        assert_eq!(tx.metadata_changes().len(), 1);
    }
}
