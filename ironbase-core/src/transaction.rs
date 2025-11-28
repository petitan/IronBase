// ironbase-core/src/transaction.rs
// Transaction management for ACD (Atomicity, Consistency, Durability)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use crate::document::DocumentId;
use crate::error::{MongoLiteError, Result};

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
        doc: Value,
    },
    /// Update an existing document
    Update {
        collection: String,
        doc_id: DocumentId,
        old_doc: Value,
        new_doc: Value,
    },
    /// Delete a document
    Delete {
        collection: String,
        doc_id: DocumentId,
        old_doc: Value, // For potential rollback
    },
}

/// Index change to be applied atomically
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexChange {
    pub operation: IndexOperation,
    pub key: IndexKey,
    pub doc_id: DocumentId,
}

/// Index operation type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexOperation {
    Insert,
    Delete,
}

/// Index key (simplified - matches index::IndexKey)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IndexKey {
    Int(i64),
    String(String),
    Float(OrderedFloat),
    Bool(bool),
    Null,
}

/// Ordered float wrapper for IndexKey
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OrderedFloat(f64);

impl OrderedFloat {
    /// Get the inner f64 value
    pub fn value(&self) -> f64 {
        self.0
    }
}

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl std::hash::Hash for OrderedFloat {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl From<&Value> for IndexKey {
    fn from(value: &Value) -> Self {
        match value {
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    IndexKey::Int(i)
                } else {
                    IndexKey::Float(OrderedFloat(n.as_f64().unwrap_or(0.0)))
                }
            }
            Value::String(s) => IndexKey::String(s.clone()),
            Value::Bool(b) => IndexKey::Bool(*b),
            Value::Null => IndexKey::Null,
            _ => IndexKey::Null, // Arrays and objects as null for now
        }
    }
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
            return Err(MongoLiteError::TransactionCommitted);
        }
        self.operations.push(op);
        Ok(())
    }

    /// Add an index change to be applied on commit
    pub fn add_index_change(&mut self, index_name: String, change: IndexChange) -> Result<()> {
        if !self.is_active() {
            return Err(MongoLiteError::TransactionCommitted);
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
            return Err(MongoLiteError::TransactionCommitted);
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
            return Err(MongoLiteError::TransactionCommitted);
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
            doc: json!({"name": "Alice"}),
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
            doc: json!({"name": "Alice"}),
        };

        assert!(matches!(
            tx.add_operation(op),
            Err(MongoLiteError::TransactionCommitted)
        ));
    }

    #[test]
    fn test_rollback() {
        let mut tx = Transaction::new(1);

        let op = Operation::Insert {
            collection: "users".to_string(),
            doc_id: DocumentId::Int(1),
            doc: json!({"name": "Alice"}),
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
