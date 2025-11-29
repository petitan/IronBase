//! IronBase Adapter - Direct wrapper around IronBase core

use crate::error::Result;
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Find options for queries
#[derive(Debug, Default)]
pub struct FindOptions {
    pub projection: Option<Value>,
    pub sort: Option<Value>,
    pub limit: Option<usize>,
    pub skip: Option<usize>,
}

/// Update result
#[derive(Debug)]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
}

/// IronBase Adapter
pub struct IronBaseAdapter {
    db: Arc<RwLock<DatabaseCore<StorageEngine>>>,
}

impl IronBaseAdapter {
    /// Create a new adapter with the given database path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = DatabaseCore::open(path)?;
        Ok(Self {
            db: Arc::new(RwLock::new(db)),
        })
    }

    // ============================================================
    // Database Management
    // ============================================================

    /// List all collections
    pub fn list_collections(&self) -> Vec<String> {
        let db = self.db.read();
        db.list_collections()
    }

    /// Drop a collection
    pub fn drop_collection(&self, name: &str) -> Result<()> {
        let db = self.db.write();
        db.drop_collection(name)?;
        Ok(())
    }

    /// Get database statistics
    pub fn stats(&self) -> Value {
        let db = self.db.read();
        serde_json::json!({
            "collections": db.list_collections(),
            "collection_count": db.list_collections().len(),
        })
    }

    /// Compact the database
    pub fn compact(&self) -> Result<Value> {
        let db = self.db.write();
        let result = db.compact()?;
        Ok(serde_json::json!({
            "size_before": result.size_before,
            "size_after": result.size_after,
            "documents_scanned": result.documents_scanned,
            "documents_kept": result.documents_kept,
            "tombstones_removed": result.tombstones_removed,
        }))
    }

    /// Force checkpoint (flush to disk)
    pub fn checkpoint(&self) -> Result<()> {
        let db = self.db.write();
        db.checkpoint()?;
        Ok(())
    }

    // ============================================================
    // Document CRUD
    // ============================================================

    /// Convert Value to HashMap for insertion
    fn value_to_hashmap(value: Value) -> HashMap<String, Value> {
        match value {
            Value::Object(map) => map.into_iter().collect(),
            _ => HashMap::new(),
        }
    }

    /// Convert DocumentId to string
    fn doc_id_to_string(id: &ironbase_core::DocumentId) -> String {
        match id {
            ironbase_core::DocumentId::Int(i) => i.to_string(),
            ironbase_core::DocumentId::String(s) => s.clone(),
            ironbase_core::DocumentId::ObjectId(oid) => oid.clone(),
        }
    }

    /// Insert a single document
    pub fn insert_one(&self, collection: &str, document: Value) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let fields = Self::value_to_hashmap(document);
        let id = coll.insert_one(fields)?;
        Ok(Self::doc_id_to_string(&id))
    }

    /// Insert multiple documents
    pub fn insert_many(&self, collection: &str, documents: Vec<Value>) -> Result<Vec<String>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let docs: Vec<HashMap<String, Value>> =
            documents.into_iter().map(Self::value_to_hashmap).collect();
        let result = coll.insert_many(docs)?;
        Ok(result
            .inserted_ids
            .iter()
            .map(Self::doc_id_to_string)
            .collect())
    }

    /// Find documents
    pub fn find(&self, collection: &str, query: Value, options: FindOptions) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;

        // Convert to IronBase FindOptions
        let ironbase_options = ironbase_core::FindOptions {
            projection: options.projection.as_ref().and_then(|p| {
                p.as_object().map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(1) as i32))
                        .collect()
                })
            }),
            sort: options.sort.as_ref().and_then(|s| {
                if let Some(arr) = s.as_array() {
                    // Array format: [["field", 1], ["field2", -1]]
                    Some(
                        arr.iter()
                            .filter_map(|item| {
                                if let Some(pair) = item.as_array() {
                                    if pair.len() == 2 {
                                        let field = pair[0].as_str()?.to_string();
                                        let dir = pair[1].as_i64()? as i32;
                                        return Some((field, dir));
                                    }
                                }
                                None
                            })
                            .collect(),
                    )
                } else {
                    // Object format: {"field": 1, "field2": -1}
                    s.as_object().map(|obj| {
                        obj.iter()
                            .map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(1) as i32))
                            .collect()
                    })
                }
            }),
            limit: options.limit,
            skip: options.skip,
        };

        let results = coll.find_with_options(&query, ironbase_options)?;
        Ok(results)
    }

    /// Find a single document
    pub fn find_one(&self, collection: &str, query: Value) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let result = coll.find_one(&query)?;
        Ok(result)
    }

    /// Update a single document
    pub fn update_one(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let (matched, modified) = coll.update_one(&filter, &update)?;
        Ok(UpdateResult {
            matched_count: matched,
            modified_count: modified,
        })
    }

    /// Update multiple documents
    pub fn update_many(
        &self,
        collection: &str,
        filter: Value,
        update: Value,
    ) -> Result<UpdateResult> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let (matched, modified) = coll.update_many(&filter, &update)?;
        Ok(UpdateResult {
            matched_count: matched,
            modified_count: modified,
        })
    }

    /// Delete a single document
    pub fn delete_one(&self, collection: &str, filter: Value) -> Result<u64> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let count = coll.delete_one(&filter)?;
        Ok(count)
    }

    /// Delete multiple documents
    pub fn delete_many(&self, collection: &str, filter: Value) -> Result<u64> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let count = coll.delete_many(&filter)?;
        Ok(count)
    }

    /// Count documents matching query
    pub fn count_documents(&self, collection: &str, query: Value) -> Result<u64> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let count = coll.count_documents(&query)?;
        Ok(count)
    }

    /// Get distinct values for a field
    pub fn distinct(&self, collection: &str, field: &str, query: Value) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let values = coll.distinct(field, &query)?;
        Ok(values)
    }

    // ============================================================
    // Aggregation
    // ============================================================

    /// Execute aggregation pipeline
    pub fn aggregate(&self, collection: &str, pipeline: Vec<Value>) -> Result<Vec<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        // Convert Vec<Value> to Value::Array
        let pipeline_value = Value::Array(pipeline);
        let results = coll.aggregate(&pipeline_value)?;
        Ok(results)
    }

    // ============================================================
    // Index Management
    // ============================================================

    /// Create an index
    pub fn create_index(&self, collection: &str, field: &str, unique: bool) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_index(field.to_string(), unique)?;
        Ok(name)
    }

    /// Create a compound index
    pub fn create_compound_index(
        &self,
        collection: &str,
        fields: &[String],
        unique: bool,
    ) -> Result<String> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let name = coll.create_compound_index(fields.to_vec(), unique)?;
        Ok(name)
    }

    /// List indexes on a collection
    pub fn list_indexes(&self, collection: &str) -> Result<Vec<String>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let indexes = coll.list_indexes();
        Ok(indexes)
    }

    /// Explain query execution plan
    pub fn explain(&self, collection: &str, query: Value) -> Result<Value> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        let plan = coll.explain(&query)?;
        Ok(plan)
    }

    // ============================================================
    // Schema Management
    // ============================================================

    /// Set schema for a collection
    pub fn set_schema(&self, collection: &str, schema: Option<Value>) -> Result<()> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        coll.set_schema(schema)?;
        Ok(())
    }

    /// Get schema for a collection
    pub fn get_schema(&self, collection: &str) -> Result<Option<Value>> {
        let db = self.db.read();
        let coll = db.collection(collection)?;
        Ok(coll.get_schema())
    }
}
