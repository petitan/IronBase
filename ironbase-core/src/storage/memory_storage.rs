// storage/memory_storage.rs
//! Pure in-memory storage implementation for fast testing
//!
//! This module provides a memory-only storage backend that implements
//! the Storage trait using HashMap. It's 10-100x faster than file-based
//! storage, perfect for unit tests.
//!
//! # Architecture
//!
//! ```text
//! MemoryStorage (Storage trait implementation)
//!      ↓
//! HashMap<String, Vec<Document>> (collections -> documents)
//! ```

use std::collections::HashMap;
use serde_json::Value;
use crate::error::{Result, MongoLiteError};
use crate::document::{Document, DocumentId};
use crate::storage::{CollectionMeta, Storage};

/// In-memory storage backend (testing)
///
/// Provides a fast, ephemeral storage implementation that stores
/// all data in memory using HashMaps. Perfect for unit tests where
/// persistence is not needed.
///
/// # Performance
///
/// - **10-100x faster** than FileStorage
/// - No file I/O overhead
/// - No persistence (data lost when dropped)
///
/// # Examples
///
/// ```ignore
/// use ironbase_core::storage::MemoryStorage;
///
/// let storage = MemoryStorage::new();
/// ```
pub struct MemoryStorage {
    /// Collection name -> documents
    collections: HashMap<String, Vec<Document>>,

    /// Collection name -> metadata
    metadata: HashMap<String, CollectionMeta>,

    /// Auto-incrementing offset counter (synthetic for compatibility)
    next_offset: u64,
}

impl MemoryStorage {
    /// Create a new empty in-memory storage
    pub fn new() -> Self {
        MemoryStorage {
            collections: HashMap::new(),
            metadata: HashMap::new(),
            next_offset: 256, // Start after header size for consistency
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage for MemoryStorage {
    // ========================================================================
    // DOCUMENT OPERATIONS
    // ========================================================================

    fn write_document(&mut self, collection: &str, doc: &Value) -> Result<u64> {
        // Parse document to extract/generate ID
        let mut doc_obj = doc.as_object()
            .ok_or_else(|| MongoLiteError::Serialization("Document must be an object".to_string()))?
            .clone();

        // Get or generate document ID
        let doc_id = if let Some(id_value) = doc_obj.get("_id") {
            // Parse existing _id
            serde_json::from_value::<DocumentId>(id_value.clone())
                .map_err(|e| MongoLiteError::Serialization(format!("Invalid _id: {}", e)))?
        } else {
            // Generate new auto-incrementing ID
            let meta = self.metadata.get_mut(collection)
                .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

            meta.last_id += 1;
            let new_id = DocumentId::Int(meta.last_id as i64);

            // Add _id to document
            let id_value = serde_json::to_value(&new_id)
                .map_err(|e| MongoLiteError::Serialization(e.to_string()))?;
            doc_obj.insert("_id".to_string(), id_value);

            new_id
        };

        // Convert to Document struct
        let doc_json = serde_json::to_string(&doc_obj)?;
        let document = Document::from_json(&doc_json)?;

        // Add to collection
        let docs = self.collections.get_mut(collection)
            .ok_or_else(|| MongoLiteError::CollectionNotFound(collection.to_string()))?;

        docs.push(document);

        // Update metadata
        let meta = self.metadata.get_mut(collection).unwrap(); // Safe: we just checked above
        meta.document_count += 1;

        // Update last_id if this is an Int ID
        if let DocumentId::Int(id) = &doc_id {
            if *id > meta.last_id as i64 {
                meta.last_id = *id as u64;
            }
        }

        // Return synthetic offset
        let offset = self.next_offset;
        self.next_offset += 1;

        // Store in catalog for compatibility
        meta.document_catalog.insert(doc_id, offset);

        Ok(offset)
    }

    fn read_document(&self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
        // Get documents for collection
        let docs = match self.collections.get(collection) {
            Some(d) => d,
            None => return Ok(None),
        };

        // Find document by ID
        for doc in docs {
            if &doc.id == id {
                // Convert Document to Value
                let doc_json = doc.to_json()?;
                let value: Value = serde_json::from_str(&doc_json)?;
                return Ok(Some(value));
            }
        }

        Ok(None)
    }

    fn scan_documents(&mut self, collection: &str) -> Result<Vec<Document>> {
        // Get documents for collection
        match self.collections.get(collection) {
            Some(docs) => {
                // Filter out tombstones
                let result = docs.iter()
                    .filter(|doc| {
                        // Check if document is a tombstone
                        doc.get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false) == false
                    })
                    .cloned()
                    .collect();
                Ok(result)
            }
            None => Ok(Vec::new()),
        }
    }

    // ========================================================================
    // COLLECTION MANAGEMENT
    // ========================================================================

    fn create_collection(&mut self, name: &str) -> Result<()> {
        if self.collections.contains_key(name) {
            return Err(MongoLiteError::CollectionExists(name.to_string()));
        }

        // Create empty collection
        self.collections.insert(name.to_string(), Vec::new());

        // Create metadata
        let meta = CollectionMeta {
            name: name.to_string(),
            document_count: 0,
            data_offset: 256, // Synthetic
            index_offset: 256, // Synthetic
            last_id: 0,
            document_catalog: HashMap::new(),
            indexes: Vec::new(),
        };

        self.metadata.insert(name.to_string(), meta);

        Ok(())
    }

    fn drop_collection(&mut self, name: &str) -> Result<()> {
        if !self.collections.contains_key(name) {
            return Err(MongoLiteError::CollectionNotFound(name.to_string()));
        }

        self.collections.remove(name);
        self.metadata.remove(name);

        Ok(())
    }

    fn list_collections(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }

    // ========================================================================
    // METADATA ACCESS
    // ========================================================================

    fn get_collection_meta(&self, name: &str) -> Option<&CollectionMeta> {
        self.metadata.get(name)
    }

    fn get_collection_meta_mut(&mut self, name: &str) -> Option<&mut CollectionMeta> {
        self.metadata.get_mut(name)
    }

    // ========================================================================
    // PERSISTENCE & FLUSHING
    // ========================================================================

    fn flush(&mut self) -> Result<()> {
        // No-op for memory storage (nothing to flush)
        Ok(())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_collection() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        assert_eq!(storage.list_collections(), vec!["users"]);
        assert!(storage.get_collection_meta("users").is_some());
    }

    #[test]
    fn test_create_duplicate_collection() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();
        let result = storage.create_collection("users");

        assert!(result.is_err());
        match result {
            Err(MongoLiteError::CollectionExists(_)) => (),
            _ => panic!("Expected CollectionExists error"),
        }
    }

    #[test]
    fn test_write_and_read_document() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        // Write document with auto-generated ID
        let doc = serde_json::json!({
            "name": "Alice",
            "age": 30
        });

        let offset = storage.write_document("users", &doc).unwrap();
        assert!(offset > 0);

        // Read document back
        let read_doc = storage.read_document("users", &DocumentId::Int(1)).unwrap();
        assert!(read_doc.is_some());

        let read_doc = read_doc.unwrap();
        assert_eq!(read_doc["name"], "Alice");
        assert_eq!(read_doc["age"], 30);
        assert_eq!(read_doc["_id"], 1);
    }

    #[test]
    fn test_write_document_with_existing_id() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        // Write document with explicit ID
        let doc = serde_json::json!({
            "_id": 42,
            "name": "Bob",
            "age": 25
        });

        storage.write_document("users", &doc).unwrap();

        // Read document back
        let read_doc = storage.read_document("users", &DocumentId::Int(42)).unwrap();
        assert!(read_doc.is_some());

        let read_doc = read_doc.unwrap();
        assert_eq!(read_doc["_id"], 42);
        assert_eq!(read_doc["name"], "Bob");
    }

    #[test]
    fn test_scan_documents() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        // Write multiple documents
        for i in 1..=5 {
            let doc = serde_json::json!({
                "name": format!("User {}", i),
                "age": 20 + i
            });
            storage.write_document("users", &doc).unwrap();
        }

        // Scan all documents
        let docs = storage.scan_documents("users").unwrap();
        assert_eq!(docs.len(), 5);

        // Verify documents (order is preserved in Vec)
        for (i, doc) in docs.iter().enumerate() {
            assert_eq!(doc.get("name").unwrap().as_str().unwrap(), format!("User {}", i + 1));
        }
    }

    #[test]
    fn test_scan_empty_collection() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("empty").unwrap();

        let docs = storage.scan_documents("empty").unwrap();
        assert_eq!(docs.len(), 0);
    }

    #[test]
    fn test_scan_nonexistent_collection() {
        let mut storage = MemoryStorage::new();

        let docs = storage.scan_documents("nonexistent").unwrap();
        assert_eq!(docs.len(), 0);
    }

    #[test]
    fn test_drop_collection() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();
        assert_eq!(storage.list_collections().len(), 1);

        storage.drop_collection("users").unwrap();
        assert_eq!(storage.list_collections().len(), 0);
    }

    #[test]
    fn test_drop_nonexistent_collection() {
        let mut storage = MemoryStorage::new();

        let result = storage.drop_collection("nonexistent");

        assert!(result.is_err());
        match result {
            Err(MongoLiteError::CollectionNotFound(_)) => (),
            _ => panic!("Expected CollectionNotFound error"),
        }
    }

    #[test]
    fn test_metadata_access() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        // Immutable access
        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.name, "users");
        assert_eq!(meta.document_count, 0);

        // Mutable access
        {
            let meta_mut = storage.get_collection_meta_mut("users").unwrap();
            meta_mut.last_id = 100;
        }

        // Verify change
        let meta = storage.get_collection_meta("users").unwrap();
        assert_eq!(meta.last_id, 100);
    }

    #[test]
    fn test_read_nonexistent_document() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();

        let doc = storage.read_document("users", &DocumentId::Int(999)).unwrap();
        assert!(doc.is_none());
    }

    #[test]
    fn test_read_from_nonexistent_collection() {
        let storage = MemoryStorage::new();

        let doc = storage.read_document("nonexistent", &DocumentId::Int(1)).unwrap();
        assert!(doc.is_none());
    }

    #[test]
    fn test_multiple_collections() {
        let mut storage = MemoryStorage::new();

        // Create multiple collections
        storage.create_collection("users").unwrap();
        storage.create_collection("posts").unwrap();
        storage.create_collection("comments").unwrap();

        assert_eq!(storage.list_collections().len(), 3);

        // Verify isolation
        let doc = serde_json::json!({"title": "Post 1"});
        storage.write_document("posts", &doc).unwrap();

        let users_docs = storage.scan_documents("users").unwrap();
        let posts_docs = storage.scan_documents("posts").unwrap();

        assert_eq!(users_docs.len(), 0);
        assert_eq!(posts_docs.len(), 1);
    }

    #[test]
    fn test_flush_is_noop() {
        let mut storage = MemoryStorage::new();

        storage.create_collection("users").unwrap();
        let doc = serde_json::json!({"name": "Alice"});
        storage.write_document("users", &doc).unwrap();

        // Flush should succeed but do nothing
        storage.flush().unwrap();

        // Data should still be accessible
        let docs = storage.scan_documents("users").unwrap();
        assert_eq!(docs.len(), 1);
    }
}
