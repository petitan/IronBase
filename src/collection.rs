// src/collection.rs
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::sync::Arc;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;

use crate::storage::StorageEngine;
use crate::document::{Document, DocumentId};
use crate::error::{Result, MongoLiteError};
use crate::query::Query;

/// Collection - dokumentumok gyűjteménye
#[pyclass]
pub struct Collection {
    name: String,
    storage: Arc<RwLock<StorageEngine>>,
}

impl Collection {
    pub fn new(name: String, storage: Arc<RwLock<StorageEngine>>) -> PyResult<Self> {
        // Collection létrehozása, ha nem létezik
        {
            let mut storage_guard = storage.write();
            if storage_guard.get_collection_meta(&name).is_none() {
                storage_guard.create_collection(&name)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
            }
        }
        
        Ok(Collection { name, storage })
    }
}

#[pymethods]
impl Collection {
    /// Egy dokumentum beszúrása
    /// 
    /// Args:
    ///     document: dict - A beszúrandó dokumentum
    /// 
    /// Returns:
    ///     dict - {"acknowledged": True, "inserted_id": ...}
    fn insert_one(&self, document: &PyDict) -> PyResult<PyObject> {
        let mut doc_map: HashMap<String, Value> = HashMap::new();
        
        // Python dict -> HashMap konverzió
        for (key, value) in document.iter() {
            let key_str: String = key.extract()?;
            let json_value = python_to_json(value)?;
            doc_map.insert(key_str, json_value);
        }
        
        // Storage írás
        let inserted_id = {
            let mut storage = self.storage.write();

            // Get mutable reference to collection metadata
            let meta = storage.get_collection_meta_mut(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            // ID generálás
            let doc_id = DocumentId::new_auto(meta.last_id);
            meta.last_id += 1;

            // Dokumentum létrehozása
            let mut doc = Document::new(doc_id.clone(), doc_map);

            // Add _collection field for multi-collection isolation
            doc.set("_collection".to_string(), Value::String(self.name.clone()));

            // Szerializálás és írás
            let doc_json = doc.to_json()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

            storage.write_data(doc_json.as_bytes())
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // Metadata (last_id) will be persisted on database close via flush()
            doc_id
        };
        
        // Eredmény visszaadása
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            
            let id_value = match inserted_id {
                DocumentId::Int(i) => i.into_py(py),
                DocumentId::String(s) => s.into_py(py),
                DocumentId::ObjectId(s) => s.into_py(py),
            };
            result.set_item("inserted_id", id_value)?;
            
            Ok(result.into())
        })
    }
    
    /// Több dokumentum beszúrása
    fn insert_many(&self, documents: &PyList) -> PyResult<PyObject> {
        let mut inserted_ids = Vec::new();
        
        for doc in documents.iter() {
            let doc_dict: &PyDict = doc.downcast()?;
            let result = self.insert_one(doc_dict)?;
            
            Python::with_gil(|py| {
                let result_dict: &PyDict = result.extract(py)?;
                let id = result_dict.get_item("inserted_id")?
                    .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("No inserted_id"))?;
                inserted_ids.push(id.to_object(py));
                Ok::<(), PyErr>(())
            })?;
        }
        
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("inserted_ids", PyList::new(py, &inserted_ids))?;
            Ok(result.into())
        })
    }
    
    /// Dokumentumok keresése
    ///
    /// Args:
    ///     query: dict - MongoDB-szerű query
    ///
    /// Returns:
    ///     list - Találatok listája
    fn find(&self, query: Option<&PyDict>) -> PyResult<PyObject> {
        // Parse query (üres query = minden dokumentum)
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Storage olvasás
        let results = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            // Use HashMap to track latest version of each document by _id
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();

            // Read from data_offset to EOF (reads ALL collections' documents)
            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            let mut current_offset = meta.data_offset;


            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // ✅ FILTER: Only include documents from THIS collection
                        let doc_collection = doc.get("_collection")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");


                        if doc_collection == self.name {
                            if let Some(id_value) = doc.get("_id") {
                                let id_key = serde_json::to_string(id_value)
                                    .unwrap_or_else(|_| "unknown".to_string());
                                docs_by_id.insert(id_key, doc);
                            }
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => {
                        break; // End of data
                    }
                }
            }

            // Now filter by query and exclude tombstones
            let mut matching_docs = Vec::new();
            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                if parsed_query.matches(&document) {
                    matching_docs.push(doc);
                }
            }

            matching_docs
        };

        // Convert to Python list
        Python::with_gil(|py| {
            let py_list = PyList::empty(py);

            for doc in results {
                let py_dict = json_to_python_dict(py, &doc)?;
                py_list.append(py_dict)?;
            }

            Ok(py_list.into())
        })
    }

    /// Egy dokumentum keresése
    ///
    /// Args:
    ///     query: dict - MongoDB-szerű query
    ///
    /// Returns:
    ///     dict vagy None
    fn find_one(&self, query: Option<&PyDict>) -> PyResult<PyObject> {
        // Parse query
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Storage olvasás
        {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // Use HashMap to track latest version of each document by _id
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // ✅ FILTER: Only include documents from THIS collection
                        let doc_collection = doc.get("_collection")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        if doc_collection == self.name {
                            if let Some(id_value) = doc.get("_id") {
                                let id_key = serde_json::to_string(id_value)
                                    .unwrap_or_else(|_| "unknown".to_string());
                                docs_by_id.insert(id_key, doc);
                            }
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Find first matching document (skip tombstones)
            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                if parsed_query.matches(&document) {
                    return Python::with_gil(|py| {
                        let py_dict = json_to_python_dict(py, &doc)?;
                        Ok(py_dict.into())
                    });
                }
            }
        }

        // No match found
        Python::with_gil(|py| Ok(py.None()))
    }

    /// Dokumentumok számlálása
    ///
    /// Args:
    ///     query: dict - MongoDB-szerű query (optional)
    ///
    /// Returns:
    ///     int - matching dokumentumok száma
    fn count_documents(&self, query: Option<&PyDict>) -> PyResult<u64> {

        // Parse query
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Storage olvasás és számlálás
        let count = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;


            // Use HashMap to track latest version of each document by _id
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // ✅ FILTER: Only include documents from THIS collection
                        let doc_collection = doc.get("_collection")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");


                        if doc_collection == self.name {
                            if let Some(id_value) = doc.get("_id") {
                                let id_key = serde_json::to_string(id_value)
                                    .unwrap_or_else(|_| "unknown".to_string());
                                docs_by_id.insert(id_key, doc);
                            }
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => {
                        break;
                    }
                }
            }


            // Count matching documents (skip tombstones)
            let mut count = 0u64;
            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                if parsed_query.matches(&document) {
                    count += 1;
                }
            }

            count
        };

        Ok(count)
    }

    /// Egyedi értékek lekérdezése egy mezőből
    ///
    /// Args:
    ///     field: str - mező neve
    ///     query: dict - MongoDB-szerű query (optional)
    ///
    /// Returns:
    ///     list - egyedi értékek listája
    fn distinct(&self, field: &str, query: Option<&PyDict>) -> PyResult<PyObject> {
        // Parse query
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Storage olvasás és egyedi értékek gyűjtése
        let distinct_values = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // Use HashMap to track latest version of each document by _id
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // ✅ FILTER: Only include documents from THIS collection
                        let doc_collection = doc.get("_collection")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        if doc_collection == self.name {
                            if let Some(id_value) = doc.get("_id") {
                                let id_key = serde_json::to_string(id_value)
                                    .unwrap_or_else(|_| "unknown".to_string());
                                docs_by_id.insert(id_key, doc);
                            }
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Collect distinct values from matching documents (skip tombstones)
            let mut seen_values: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut distinct_values = Vec::new();

            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                // Check if matches query
                if parsed_query.matches(&document) {
                    // Extract field value
                    if let Some(field_value) = doc.get(field) {
                        // Use JSON string representation for uniqueness check
                        let value_key = serde_json::to_string(field_value)
                            .unwrap_or_else(|_| "null".to_string());

                        // Only add if not seen before
                        if seen_values.insert(value_key) {
                            distinct_values.push(field_value.clone());
                        }
                    }
                }
            }

            distinct_values
        };

        // Convert to Python list
        Python::with_gil(|py| {
            let py_list = PyList::empty(py);
            for value in distinct_values {
                let py_value = json_value_to_python(py, &value)?;
                py_list.append(py_value)?;
            }
            Ok(py_list.into())
        })
    }

    /// Dokumentum frissítése
    fn update_one(&self, query: &PyDict, update: &PyDict) -> PyResult<PyObject> {
        // Parse query
        let query_json = python_dict_to_json_value(query)?;
        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Parse update operators
        let update_json = python_dict_to_json_value(update)?;

        // Find and update first matching document (using HashMap to get latest version)
        let (matched_count, modified_count) = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // First pass: collect all documents by _id (latest version only)
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Track latest version (include tombstones so they overwrite originals)
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Second pass: find first matching and update (skip tombstones)
            let mut matched = 0u64;
            let mut modified = 0u64;

            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }
                if matched > 0 {
                    break; // Only update first match
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let mut document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                // Check if matches query
                if parsed_query.matches(&document) {
                    matched = 1;

                    // Apply update operators
                    let mut was_modified = false;
                    if let Value::Object(ref update_ops) = update_json {
                        for (op, fields) in update_ops {
                            match op.as_str() {
                                "$set" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, value) in field_values {
                                            document.set(field.clone(), value.clone());
                                            was_modified = true;
                                        }
                                    }
                                }
                                "$inc" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, inc_value) in field_values {
                                            if let Some(current) = document.get(field) {
                                                // Try int first to preserve integer types
                                                if let (Some(curr_int), Some(inc_int)) = (current.as_i64(), inc_value.as_i64()) {
                                                    document.set(field.clone(), Value::from(curr_int + inc_int));
                                                    was_modified = true;
                                                } else if let (Some(curr_num), Some(inc_num)) = (current.as_f64(), inc_value.as_f64()) {
                                                    document.set(field.clone(), Value::from(curr_num + inc_num));
                                                    was_modified = true;
                                                }
                                            }
                                        }
                                    }
                                }
                                "$unset" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, _) in field_values {
                                            document.remove(field);
                                            was_modified = true;
                                        }
                                    }
                                }
                                _ => {
                                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                                        format!("Unsupported update operator: {}", op)
                                    ));
                                }
                            }
                        }
                    }

                    if was_modified {
                        // Mark old document as tombstone
                        let mut tombstone = doc.clone();
                        if let Value::Object(ref mut map) = tombstone {
                            map.insert("_tombstone".to_string(), Value::Bool(true));
                            map.insert("_collection".to_string(), Value::String(self.name.clone())); // ✅ Ensure _collection
                        }
                        let tombstone_json = serde_json::to_string(&tombstone)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Write tombstone
                        storage.write_data(tombstone_json.as_bytes())
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                        // ✅ Ensure updated document has _collection
                        document.set("_collection".to_string(), Value::String(self.name.clone()));

                        // Write updated document
                        let updated_json = document.to_json()
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                        storage.write_data(updated_json.as_bytes())
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                        modified = 1;
                    }
                }
            }

            (matched, modified)
        };

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("matched_count", matched_count)?;
            result.set_item("modified_count", modified_count)?;
            Ok(result.into())
        })
    }
    
    /// Több dokumentum frissítése
    fn update_many(&self, query: &PyDict, update: &PyDict) -> PyResult<PyObject> {
        // Parse query
        let query_json = python_dict_to_json_value(query)?;
        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Parse update operators
        let update_json = python_dict_to_json_value(update)?;

        // Find and update ALL matching documents (using HashMap to get latest version)
        let (matched_count, modified_count) = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // First pass: collect all documents by _id (latest version only)
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Track latest version (include tombstones so they overwrite originals)
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Second pass: find all matching and update (skip tombstones)
            let mut matched = 0u64;
            let mut modified = 0u64;

            for (_, doc) in docs_by_id {
                // Skip tombstones (deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let mut document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                // Check if matches query
                if parsed_query.matches(&document) {
                    matched += 1;

                    // Apply update operators
                    let mut was_modified = false;
                    if let Value::Object(ref update_ops) = update_json {
                        for (op, fields) in update_ops {
                            match op.as_str() {
                                "$set" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, value) in field_values {
                                            document.set(field.clone(), value.clone());
                                            was_modified = true;
                                        }
                                    }
                                }
                                "$inc" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, inc_value) in field_values {
                                            if let Some(current) = document.get(field) {
                                                // Try int first to preserve integer types
                                                if let (Some(curr_int), Some(inc_int)) = (current.as_i64(), inc_value.as_i64()) {
                                                    document.set(field.clone(), Value::from(curr_int + inc_int));
                                                    was_modified = true;
                                                } else if let (Some(curr_num), Some(inc_num)) = (current.as_f64(), inc_value.as_f64()) {
                                                    document.set(field.clone(), Value::from(curr_num + inc_num));
                                                    was_modified = true;
                                                }
                                            }
                                        }
                                    }
                                }
                                "$unset" => {
                                    if let Value::Object(ref field_values) = fields {
                                        for (field, _) in field_values {
                                            document.remove(field);
                                            was_modified = true;
                                        }
                                    }
                                }
                                _ => {
                                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                                        format!("Unsupported update operator: {}", op)
                                    ));
                                }
                            }
                        }
                    }

                    if was_modified {
                        // Mark old document as tombstone
                        let mut tombstone = doc.clone();
                        if let Value::Object(ref mut map) = tombstone {
                            map.insert("_tombstone".to_string(), Value::Bool(true));
                            map.insert("_collection".to_string(), Value::String(self.name.clone())); // ✅ Ensure _collection
                        }
                        let tombstone_json = serde_json::to_string(&tombstone)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Write tombstone
                        storage.write_data(tombstone_json.as_bytes())
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                        // ✅ Ensure updated document has _collection
                        document.set("_collection".to_string(), Value::String(self.name.clone()));

                        // Write updated document
                        let updated_json = document.to_json()
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                        storage.write_data(updated_json.as_bytes())
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                        modified += 1;
                    }
                }
            }

            (matched, modified)
        };

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("matched_count", matched_count)?;
            result.set_item("modified_count", modified_count)?;
            Ok(result.into())
        })
    }
    
    /// Dokumentum törlése
    fn delete_one(&self, query: &PyDict) -> PyResult<PyObject> {
        // Parse query
        let query_json = python_dict_to_json_value(query)?;
        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Find and delete first matching document (using HashMap to get latest version)
        let deleted_count = {
            let mut storage = self.storage.write();
            let mut meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?
                .clone();

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // First pass: collect all documents by _id (latest version only)
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Track latest version (include tombstones so they overwrite originals)
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Second pass: find first matching and delete (skip tombstones)
            let mut deleted = 0u64;

            for (_, doc) in docs_by_id {
                // Skip tombstones (already deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }
                if deleted > 0 {
                    break; // Only delete first match
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                // Check if matches query
                if parsed_query.matches(&document) {
                    // Mark as tombstone (logical delete)
                    let mut tombstone = doc.clone();
                    if let Value::Object(ref mut map) = tombstone {
                        map.insert("_tombstone".to_string(), Value::Bool(true));
                        map.insert("_collection".to_string(), Value::String(self.name.clone())); // ✅ Ensure _collection
                    }
                    let tombstone_json = serde_json::to_string(&tombstone)
                        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                    // Write tombstone
                    storage.write_data(tombstone_json.as_bytes())
                        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                    deleted = 1;
                }
            }

            deleted
        };

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", deleted_count)?;
            Ok(result.into())
        })
    }
    
    /// Több dokumentum törlése
    fn delete_many(&self, query: &PyDict) -> PyResult<PyObject> {
        // Parse query
        let query_json = python_dict_to_json_value(query)?;
        let parsed_query = Query::from_json(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

        // Find and delete ALL matching documents (using HashMap to get latest version)
        let deleted_count = {
            let mut storage = self.storage.write();
            let meta = storage.get_collection_meta(&self.name)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Collection not found"))?;

            let file_len = storage.file_len()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

            // First pass: collect all documents by _id (latest version only)
            let mut docs_by_id: HashMap<String, Value> = HashMap::new();
            let mut current_offset = meta.data_offset;

            while current_offset < file_len {
                match storage.read_data(current_offset) {
                    Ok(doc_bytes) => {
                        let doc: Value = serde_json::from_slice(&doc_bytes)
                            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                        // Track latest version (include tombstones so they overwrite originals)
                        if let Some(id_value) = doc.get("_id") {
                            let id_key = serde_json::to_string(id_value)
                                .unwrap_or_else(|_| "unknown".to_string());
                            docs_by_id.insert(id_key, doc);
                        }

                        current_offset += 4 + doc_bytes.len() as u64;
                    }
                    Err(_) => break,
                }
            }

            // Second pass: find all matching and delete (skip tombstones)
            let mut deleted = 0u64;

            for (_, doc) in docs_by_id {
                // Skip tombstones (already deleted documents)
                if doc.get("_tombstone").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }

                let doc_json_str = serde_json::to_string(&doc)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;
                let document = Document::from_json(&doc_json_str)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                // Check if matches query
                if parsed_query.matches(&document) {
                    // Mark as tombstone (logical delete)
                    let mut tombstone = doc.clone();
                    if let Value::Object(ref mut map) = tombstone {
                        map.insert("_tombstone".to_string(), Value::Bool(true));
                        map.insert("_collection".to_string(), Value::String(self.name.clone())); // ✅ Ensure _collection
                    }
                    let tombstone_json = serde_json::to_string(&tombstone)
                        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

                    // Write tombstone
                    storage.write_data(tombstone_json.as_bytes())
                        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

                    deleted += 1;
                }
            }

            deleted
        };

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", deleted_count)?;
            Ok(result.into())
        })
    }

    fn __repr__(&self) -> String {
        format!("Collection('{}')", self.name)
    }
}

/// Python érték -> JSON konverzió
fn python_to_json(value: &PyAny) -> PyResult<Value> {
    if value.is_none() {
        Ok(Value::Null)
    } else if let Ok(b) = value.extract::<bool>() {
        Ok(Value::Bool(b))
    } else if let Ok(i) = value.extract::<i64>() {
        Ok(Value::Number(i.into()))
    } else if let Ok(f) = value.extract::<f64>() {
        Ok(serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null))
    } else if let Ok(s) = value.extract::<String>() {
        Ok(Value::String(s))
    } else if let Ok(list) = value.downcast::<PyList>() {
        let mut arr = Vec::new();
        for item in list.iter() {
            arr.push(python_to_json(item)?);
        }
        Ok(Value::Array(arr))
    } else if let Ok(dict) = value.downcast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            map.insert(key, python_to_json(v)?);
        }
        Ok(Value::Object(map))
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            format!("Unsupported type: {:?}", value.get_type())
        ))
    }
}

/// Python dict -> JSON Value konverzió
fn python_dict_to_json_value(dict: &PyDict) -> PyResult<Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in dict.iter() {
        let key: String = k.extract()?;
        map.insert(key, python_to_json(v)?);
    }
    Ok(Value::Object(map))
}

/// JSON Value -> Python dict konverzió
fn json_to_python_dict<'a>(py: Python<'a>, value: &Value) -> PyResult<&'a PyDict> {
    let dict = PyDict::new(py);

    if let Value::Object(map) = value {
        for (key, val) in map.iter() {
            let py_val = json_value_to_python(py, val)?;
            dict.set_item(key, py_val)?;
        }
    }

    Ok(dict)
}

/// JSON Value -> Python value konverzió
fn json_value_to_python(py: Python, value: &Value) -> PyResult<PyObject> {
    match value {
        Value::Null => Ok(py.None()),
        Value::Bool(b) => Ok(b.into_py(py)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_py(py))
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_py(py))
            } else {
                Ok(py.None())
            }
        }
        Value::String(s) => Ok(s.into_py(py)),
        Value::Array(arr) => {
            let py_list = PyList::empty(py);
            for item in arr {
                py_list.append(json_value_to_python(py, item)?)?;
            }
            Ok(py_list.into())
        }
        Value::Object(map) => {
            let py_dict = PyDict::new(py);
            for (k, v) in map.iter() {
                py_dict.set_item(k, json_value_to_python(py, v)?)?;
            }
            Ok(py_dict.into())
        }
    }
}
