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

use crate::document::{Document, DocumentId};
use crate::error::{MongoLiteError, Result};
use crate::storage::{CollectionMeta, RawStorage, Storage};
use serde_json::Value;
use std::collections::HashMap;

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

    /// Raw byte buffer for RawStorage trait (simulates file storage)
    raw_data: Vec<u8>,
}

impl MemoryStorage {
    /// Create a new empty in-memory storage
    pub fn new() -> Self {
        // Pre-allocate header space (256 bytes) to match file-based storage
        let mut raw_data = vec![0u8; 256];
        // Write magic header for consistency
        raw_data[0..8].copy_from_slice(b"MONGOLTE");

        MemoryStorage {
            collections: HashMap::new(),
            metadata: HashMap::new(),
            next_offset: 256, // Start after header size for consistency
            raw_data,
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
        let mut doc_obj = doc
            .as_object()
            .ok_or_else(|| MongoLiteError::Serialization("Document must be an object".to_string()))?
            .clone();

        // Get or generate document ID
        let doc_id = if let Some(id_value) = doc_obj.get("_id") {
            // Parse existing _id
            serde_json::from_value::<DocumentId>(id_value.clone())
                .map_err(|e| MongoLiteError::Serialization(format!("Invalid _id: {}", e)))?
        } else {
            // Generate new auto-incrementing ID
            let meta = self
                .metadata
                .get_mut(collection)
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
        let docs = self
            .collections
            .get_mut(collection)
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

    fn read_document(&mut self, collection: &str, id: &DocumentId) -> Result<Option<Value>> {
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
                let result = docs
                    .iter()
                    .filter(|doc| {
                        // Check if document is a tombstone
                        doc.get("_tombstone")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            == false
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
            live_document_count: 0,
            data_offset: 256,  // Synthetic
            index_offset: 256, // Synthetic
            last_id: 0,
            document_catalog: HashMap::new(),
            indexes: Vec::new(),
            schema: None,
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

    fn checkpoint(&mut self) -> Result<()> {
        // No-op for memory storage (no WAL)
        Ok(())
    }

    fn adjust_live_count(&mut self, collection: &str, delta: i64) {
        if let Some(meta) = self.metadata.get_mut(collection) {
            if delta >= 0 {
                meta.live_document_count = meta.live_document_count.saturating_add(delta as u64);
            } else {
                let dec = (-delta) as u64;
                meta.live_document_count = meta.live_document_count.saturating_sub(dec);
            }
        }
    }

    fn get_live_count(&self, collection: &str) -> Option<u64> {
        self.metadata.get(collection).map(|m| m.live_document_count)
    }

    fn get_file_path(&self) -> &str {
        ""
    }
}

// ============================================================================
// RAW STORAGE IMPLEMENTATION
// ============================================================================

impl RawStorage for MemoryStorage {
    /// Write raw document bytes with catalog tracking
    ///
    /// Format: [u32 length][raw bytes]
    fn write_document_raw(
        &mut self,
        collection: &str,
        doc_id: &DocumentId,
        data: &[u8],
    ) -> Result<u64> {
        let offset = self.raw_data.len() as u64;

        // Write length prefix (4 bytes, little-endian)
        let len = data.len() as u32;
        self.raw_data.extend_from_slice(&len.to_le_bytes());

        // Write raw document bytes
        self.raw_data.extend_from_slice(data);

        // Update catalog in metadata
        if let Some(meta) = self.metadata.get_mut(collection) {
            meta.document_catalog.insert(doc_id.clone(), offset);
        }

        // Update next_offset for consistency
        self.next_offset = self.raw_data.len() as u64;

        Ok(offset)
    }

    /// Read document bytes at specific offset
    fn read_document_at(&mut self, _collection: &str, offset: u64) -> Result<Vec<u8>> {
        let start = offset as usize;

        // Check bounds
        if start + 4 > self.raw_data.len() {
            return Err(MongoLiteError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("Offset {} out of bounds (len={})", offset, self.raw_data.len()),
            )));
        }

        // Read length prefix
        let len_bytes: [u8; 4] = self.raw_data[start..start + 4]
            .try_into()
            .map_err(|_| MongoLiteError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Failed to read length prefix",
            )))?;
        let len = u32::from_le_bytes(len_bytes) as usize;

        // Check data bounds
        if start + 4 + len > self.raw_data.len() {
            return Err(MongoLiteError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("Data extends beyond buffer: offset={}, len={}", offset, len),
            )));
        }

        // Return raw bytes (without length prefix)
        Ok(self.raw_data[start + 4..start + 4 + len].to_vec())
    }

    /// Write raw data without catalog tracking (for tombstones)
    fn write_data(&mut self, data: &[u8]) -> Result<u64> {
        let offset = self.raw_data.len() as u64;

        // Write length prefix (4 bytes, little-endian)
        let len = data.len() as u32;
        self.raw_data.extend_from_slice(&len.to_le_bytes());

        // Write raw bytes
        self.raw_data.extend_from_slice(data);

        // Update next_offset
        self.next_offset = self.raw_data.len() as u64;

        Ok(offset)
    }

    /// Read raw data at offset
    fn read_data(&mut self, offset: u64) -> Result<Vec<u8>> {
        // Same implementation as read_document_at (no collection context needed)
        self.read_document_at("", offset)
    }

    /// Get current "file" length (raw_data buffer size)
    fn file_len(&self) -> Result<u64> {
        Ok(self.raw_data.len() as u64)
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
        let read_doc = storage
            .read_document("users", &DocumentId::Int(42))
            .unwrap();
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
            assert_eq!(
                doc.get("name").unwrap().as_str().unwrap(),
                format!("User {}", i + 1)
            );
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

        let doc = storage
            .read_document("users", &DocumentId::Int(999))
            .unwrap();
        assert!(doc.is_none());
    }

    #[test]
    fn test_read_from_nonexistent_collection() {
        let mut storage = MemoryStorage::new();

        let doc = storage
            .read_document("nonexistent", &DocumentId::Int(1))
            .unwrap();
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

    // ========== RawStorage Tests ==========

    #[test]
    fn test_raw_storage_write_and_read() {
        let mut storage = MemoryStorage::new();
        storage.create_collection("test").unwrap();

        let doc_id = DocumentId::Int(1);
        let data = b"{\"name\":\"Alice\"}";

        let offset = storage
            .write_document_raw("test", &doc_id, data)
            .unwrap();

        // Offset should be after header (256 bytes)
        assert_eq!(offset, 256);

        // Read back
        let read_data = storage.read_document_at("test", offset).unwrap();
        assert_eq!(read_data, data);
    }

    #[test]
    fn test_raw_storage_multiple_writes() {
        let mut storage = MemoryStorage::new();
        storage.create_collection("test").unwrap();

        let data1 = b"first document";
        let data2 = b"second document";

        let offset1 = storage
            .write_document_raw("test", &DocumentId::Int(1), data1)
            .unwrap();
        let offset2 = storage
            .write_document_raw("test", &DocumentId::Int(2), data2)
            .unwrap();

        // Second offset should be after first data
        assert!(offset2 > offset1);

        // Read both back
        assert_eq!(storage.read_document_at("test", offset1).unwrap(), data1);
        assert_eq!(storage.read_document_at("test", offset2).unwrap(), data2);
    }

    #[test]
    fn test_raw_storage_write_data() {
        let mut storage = MemoryStorage::new();

        let tombstone = b"{\"_tombstone\":true}";
        let offset = storage.write_data(tombstone).unwrap();

        // Should be able to read back
        let read_data = storage.read_data(offset).unwrap();
        assert_eq!(read_data, tombstone);
    }

    #[test]
    fn test_raw_storage_file_len() {
        let mut storage = MemoryStorage::new();

        // Initial length is header size (256)
        assert_eq!(storage.file_len().unwrap(), 256);

        // Write some data
        storage.write_data(b"test").unwrap();

        // Length should have increased: 256 + 4 (len prefix) + 4 (data)
        assert_eq!(storage.file_len().unwrap(), 264);
    }

    #[test]
    fn test_raw_storage_read_invalid_offset() {
        let mut storage = MemoryStorage::new();

        // Try to read beyond buffer
        let result = storage.read_document_at("test", 9999);
        assert!(result.is_err());
    }

    #[test]
    fn test_raw_storage_catalog_update() {
        let mut storage = MemoryStorage::new();
        storage.create_collection("test").unwrap();

        let doc_id = DocumentId::String("doc1".to_string());
        let data = b"{\"_id\":\"doc1\"}";

        let offset = storage
            .write_document_raw("test", &doc_id, data)
            .unwrap();

        // Check catalog was updated
        let meta = storage.get_collection_meta("test").unwrap();
        assert_eq!(meta.document_catalog.get(&doc_id), Some(&offset));
    }
}
