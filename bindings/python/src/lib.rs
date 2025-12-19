// bindings/python/src/lib.rs
// PyO3 0.24 wrapper for ironbase-core

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use ironbase_core::{CollectionCore, DatabaseCore, DocumentId, DurabilityMode, StorageEngine};

/// IronBase Database - Python wrapper
#[pyclass]
pub struct IronBase {
    db: Arc<DatabaseCore<StorageEngine>>,
}

#[pymethods]
impl IronBase {
    /// Create or open a database
    #[new]
    #[pyo3(signature = (path, durability="safe", batch_size=100, auto_checkpoint=None))]
    fn new(
        path: String,
        durability: &str,
        batch_size: usize,
        auto_checkpoint: Option<usize>,
    ) -> PyResult<Self> {
        let mode = match durability {
            "safe" => DurabilityMode::Safe,
            "batch" => DurabilityMode::Batch { batch_size },
            "unsafe" => {
                if let Some(checkpoint_ops) = auto_checkpoint {
                    DurabilityMode::unsafe_auto(checkpoint_ops)
                } else {
                    DurabilityMode::unsafe_manual()
                }
            }
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid durability mode '{}'. Must be 'safe', 'batch', or 'unsafe'",
                    durability
                )));
            }
        };

        let db = DatabaseCore::open_with_durability(&path, mode)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        Ok(IronBase { db: Arc::new(db) })
    }

    /// Get or create a collection
    fn collection(&self, name: String) -> PyResult<Collection> {
        let coll_core = self
            .db
            .collection(&name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(Collection {
            core: coll_core,
            db: Arc::clone(&self.db),
            name: name.clone(),
        })
    }

    /// List all collections
    fn list_collections(&self) -> PyResult<Vec<String>> {
        Ok(self.db.list_collections())
    }

    /// Set or clear JSON schema for a collection
    fn set_collection_schema(
        &self,
        py: Python<'_>,
        name: String,
        schema: Option<Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let schema_json = match schema {
            Some(dict) => Some(python_dict_to_json_value(py, &dict)?),
            None => None,
        };

        self.db
            .set_collection_schema(&name, schema_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Drop a collection
    fn drop_collection(&self, name: String) -> PyResult<()> {
        self.db
            .drop_collection(&name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Close the database: flush all changes and release the file lock.
    ///
    /// After calling close(), another process can open the same database file.
    /// The database instance should not be used after calling close().
    fn close(&self) -> PyResult<()> {
        self.db
            .close()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))
    }

    /// Checkpoint - Clear WAL
    fn checkpoint(&self) -> PyResult<()> {
        self.db
            .checkpoint()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))
    }

    /// Get database statistics
    fn stats(&self) -> PyResult<String> {
        Ok(serde_json::to_string_pretty(&self.db.stats()).unwrap())
    }

    /// Set global log level
    #[staticmethod]
    fn set_log_level(level: String) -> PyResult<()> {
        let log_level = ironbase_core::LogLevel::from_str(&level).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Invalid log level '{}'. Must be one of: ERROR, WARN, INFO, DEBUG, TRACE",
                level
            ))
        })?;

        ironbase_core::set_log_level(log_level);
        Ok(())
    }

    /// Get current log level
    #[staticmethod]
    fn get_log_level() -> PyResult<String> {
        let level = ironbase_core::get_log_level();
        Ok(level.as_str().to_string())
    }

    /// Storage compaction
    fn compact<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let stats = self
            .db
            .compact()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let dict = PyDict::new(py);
        dict.set_item("size_before", stats.size_before)?;
        dict.set_item("size_after", stats.size_after)?;
        dict.set_item("space_saved", stats.space_saved())?;
        dict.set_item("documents_scanned", stats.documents_scanned)?;
        dict.set_item("documents_kept", stats.documents_kept)?;
        dict.set_item("tombstones_removed", stats.tombstones_removed)?;
        dict.set_item("peak_memory_mb", stats.peak_memory_mb)?;
        dict.set_item("compression_ratio", stats.compression_ratio())?;
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!("IronBase('{}')", self.db.path())
    }

    // ========== ACD TRANSACTION API ==========

    /// Begin a new transaction
    fn begin_transaction(&self) -> PyResult<u64> {
        Ok(self.db.begin_transaction())
    }

    /// Commit a transaction
    fn commit_transaction(&self, tx_id: u64) -> PyResult<()> {
        self.db
            .commit_transaction(tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Rollback a transaction
    fn rollback_transaction(&self, tx_id: u64) -> PyResult<()> {
        self.db
            .rollback_transaction(tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Insert one document within a transaction
    fn insert_one_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        document: Bound<'_, PyDict>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut doc_map: HashMap<String, Value> = HashMap::new();
        for (key, value) in document.iter() {
            let key_str: String = key.extract()?;
            let json_value = python_to_json(py, &value)?;
            doc_map.insert(key_str, json_value);
        }

        let inserted_id = self
            .db
            .insert_one_tx(&collection_name, doc_map, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        let id_value = doc_id_to_py(py, &inserted_id)?;
        result.set_item("inserted_id", id_value)?;
        Ok(result)
    }

    /// Update one document within a transaction
    fn update_one_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        query: Bound<'_, PyDict>,
        new_doc: Bound<'_, PyDict>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;
        let new_doc_json = python_dict_to_json_value(py, &new_doc)?;

        let (matched_count, modified_count) = self
            .db
            .update_one_tx(&collection_name, &query_json, new_doc_json, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("matched_count", matched_count)?;
        result.set_item("modified_count", modified_count)?;
        Ok(result)
    }

    /// Delete one document within a transaction
    fn delete_one_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        query: Bound<'_, PyDict>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        let deleted_count = self
            .db
            .delete_one_tx(&collection_name, &query_json, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }
}

/// Collection wrapper
#[pyclass]
pub struct Collection {
    core: CollectionCore<StorageEngine>,
    db: Arc<DatabaseCore<StorageEngine>>,
    name: String,
}

#[pymethods]
impl Collection {
    /// Set or clear JSON schema
    fn set_schema(&self, py: Python<'_>, schema: Option<Bound<'_, PyDict>>) -> PyResult<()> {
        let schema_json = match schema {
            Some(dict) => Some(python_dict_to_json_value(py, &dict)?),
            None => None,
        };

        self.core
            .set_schema(schema_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Get current JSON schema
    fn get_schema<'py>(&self, py: Python<'py>) -> PyResult<PyObject> {
        let schema = self
            .core
            .get_schema()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;
        match schema {
            Some(v) => json_value_to_python(py, &v),
            None => Ok(py.None()),
        }
    }

    /// Insert one document
    fn insert_one<'py>(
        &self,
        py: Python<'py>,
        document: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut doc_map: HashMap<String, Value> = HashMap::new();

        for (key, value) in document.iter() {
            let key_str: String = key.extract()?;
            let json_value = python_to_json(py, &value)?;
            doc_map.insert(key_str, json_value);
        }

        let inserted_id = self
            .db
            .insert_one(&self.name, doc_map)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        let id_value = doc_id_to_py(py, &inserted_id)?;
        result.set_item("inserted_id", id_value)?;
        Ok(result)
    }

    /// Insert many documents
    fn insert_many<'py>(
        &self,
        py: Python<'py>,
        documents: Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut docs = Vec::with_capacity(documents.len());
        for doc in documents.iter() {
            let doc_dict = doc.downcast::<PyDict>()?;
            let mut fields = HashMap::new();

            for (key, value) in doc_dict.iter() {
                let key_str: String = key.extract()?;
                let value_json = python_to_json(py, &value)?;
                fields.insert(key_str, value_json);
            }

            docs.push(fields);
        }

        let inserted_ids = self
            .db
            .insert_many(&self.name, docs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result_dict = PyDict::new(py);
        result_dict.set_item("acknowledged", true)?;
        result_dict.set_item("inserted_count", inserted_ids.len())?;

        let ids_list = PyList::empty(py);
        for doc_id in inserted_ids {
            let id_value = doc_id_to_py(py, &doc_id)?;
            ids_list.append(id_value)?;
        }
        result_dict.set_item("inserted_ids", ids_list)?;

        Ok(result_dict)
    }

    /// Find documents with options
    #[pyo3(signature = (query=None, projection=None, sort=None, limit=None, skip=None))]
    fn find<'py>(
        &self,
        py: Python<'py>,
        query: Option<Bound<'_, PyDict>>,
        projection: Option<Bound<'_, PyDict>>,
        sort: Option<Bound<'_, PyList>>,
        limit: Option<usize>,
        skip: Option<usize>,
    ) -> PyResult<Bound<'py, PyList>> {
        use ironbase_core::find_options::FindOptions;

        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        let mut options = FindOptions::new();

        if let Some(proj) = projection {
            let mut projection_map = HashMap::new();
            for (key, value) in proj.iter() {
                let field: String = key.extract()?;
                let action: i32 = value.extract()?;
                projection_map.insert(field, action);
            }
            options.projection = Some(projection_map);
        }

        if let Some(sort_list) = sort {
            let mut sort_vec = Vec::new();
            for item in sort_list.iter() {
                let tuple = item.downcast::<PyTuple>()?;
                let field: String = tuple.get_item(0)?.extract()?;
                let direction: i32 = tuple.get_item(1)?.extract()?;
                sort_vec.push((field, direction));
            }
            options.sort = Some(sort_vec);
        }

        options.limit = limit;
        options.skip = skip;

        let results = self
            .core
            .find_with_options(&query_json, options)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let py_list = PyList::empty(py);
        for doc in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Find one document
    fn find_one<'py>(
        &self,
        py: Python<'py>,
        query: Option<Bound<'_, PyDict>>,
    ) -> PyResult<PyObject> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        let result = self
            .core
            .find_one(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        match result {
            Some(doc) => {
                let py_dict = json_to_python_dict(py, &doc)?;
                Ok(py_dict.into_any().unbind())
            }
            None => Ok(py.None()),
        }
    }

    /// Count documents
    fn count_documents(&self, py: Python<'_>, query: Option<Bound<'_, PyDict>>) -> PyResult<u64> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        self.core
            .count_documents(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Distinct values
    fn distinct<'py>(
        &self,
        py: Python<'py>,
        field: &str,
        query: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        let distinct_values = self
            .core
            .distinct(field, &query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let py_list = PyList::empty(py);
        for value in distinct_values {
            let py_value = json_value_to_python(py, &value)?;
            py_list.append(py_value)?;
        }
        Ok(py_list)
    }

    /// Update one document
    fn update_one<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
        update: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;
        let update_json = python_dict_to_json_value(py, &update)?;

        let (matched_count, modified_count) = self
            .db
            .update_one(&self.name, &query_json, &update_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("matched_count", matched_count)?;
        result.set_item("modified_count", modified_count)?;
        Ok(result)
    }

    /// Update many documents
    fn update_many<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
        update: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;
        let update_json = python_dict_to_json_value(py, &update)?;

        let (matched_count, modified_count) = self
            .db
            .update_many(&self.name, &query_json, &update_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("matched_count", matched_count)?;
        result.set_item("modified_count", modified_count)?;
        Ok(result)
    }

    /// Delete one document
    fn delete_one<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        let deleted_count = self
            .db
            .delete_one(&self.name, &query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }

    /// Delete many documents
    fn delete_many<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        let deleted_count = self
            .db
            .delete_many(&self.name, &query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }

    /// Create an index
    #[pyo3(signature = (field, unique=false))]
    fn create_index(&self, field: String, unique: bool) -> PyResult<String> {
        self.core
            .create_index(field, unique)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Create a compound index
    #[pyo3(signature = (fields, unique=false))]
    fn create_compound_index(&self, fields: Vec<String>, unique: bool) -> PyResult<String> {
        if fields.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Compound index must have at least one field",
            ));
        }

        self.core
            .create_compound_index(fields, unique)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Drop an index
    fn drop_index(&self, index_name: String) -> PyResult<()> {
        self.core
            .drop_index(&index_name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// List all indexes
    fn list_indexes(&self) -> PyResult<Vec<String>> {
        Ok(self.core.list_indexes())
    }

    /// Explain query
    fn explain<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        let plan = self
            .core
            .explain(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        json_to_python_dict(py, &plan)
    }

    /// Find with hint
    fn find_with_hint<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
        hint: String,
    ) -> PyResult<Bound<'py, PyList>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        let results = self
            .core
            .find_with_hint(&query_json, &hint)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let py_list = PyList::empty(py);
        for doc in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Execute aggregation pipeline
    fn aggregate<'py>(
        &self,
        py: Python<'py>,
        pipeline: Bound<'_, PyList>,
    ) -> PyResult<Bound<'py, PyList>> {
        let mut stages = Vec::new();
        for stage in pipeline.iter() {
            let stage_dict = stage.downcast::<PyDict>()?;
            let stage_json = python_dict_to_json_value(py, stage_dict)?;
            stages.push(stage_json);
        }

        let pipeline_json = serde_json::Value::Array(stages);

        let results = self
            .core
            .aggregate(&pipeline_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let py_list = PyList::empty(py);
        for doc in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Create a lazy cursor for streaming large result sets
    ///
    /// Unlike find(), this does NOT load all documents into memory.
    /// Documents are fetched in batches as you iterate.
    ///
    /// # Example
    /// ```python
    /// cursor = collection.find_cursor({"status": "active"}, batch_size=100)
    /// for doc in cursor:
    ///     process(doc)  # Documents loaded batch by batch
    /// ```
    #[pyo3(signature = (query=None, batch_size=100))]
    fn find_cursor(
        &self,
        py: Python<'_>,
        query: Option<Bound<'_, PyDict>>,
        batch_size: usize,
    ) -> PyResult<Cursor> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        // Don't load documents yet - lazy loading on iteration
        Ok(Cursor {
            db: Arc::clone(&self.db),
            collection_name: self.name.clone(),
            query: query_json,
            position: 0,
            batch_size,
            exhausted: false,
            // Local buffer for current batch
            current_batch: Vec::new(),
            batch_position: 0,
        })
    }

    fn __repr__(&self) -> String {
        format!("Collection('{}')", self.core.name)
    }
}

/// Lazy cursor for iterating through query results
///
/// Documents are loaded in batches as you iterate, not all at once.
/// This allows processing millions of documents without memory exhaustion.
#[pyclass]
pub struct Cursor {
    db: Arc<DatabaseCore<StorageEngine>>,
    collection_name: String,
    query: Value,
    position: usize, // Global position (skip offset for DB query)
    batch_size: usize,
    exhausted: bool,
    // Local buffer for current batch
    current_batch: Vec<Value>,
    batch_position: usize, // Position within current_batch
}

impl Cursor {
    /// Fetch the next batch from database
    fn fetch_next_batch(&mut self) -> PyResult<()> {
        if self.exhausted {
            return Ok(());
        }

        let collection = self
            .db
            .collection(&self.collection_name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        let options = ironbase_core::FindOptions::new()
            .with_skip(self.position)
            .with_limit(self.batch_size);

        let results = collection
            .find_with_options(&self.query, options)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // If we got fewer results than batch_size, we've exhausted the data
        if results.len() < self.batch_size {
            self.exhausted = true;
        }

        self.position += results.len();
        self.current_batch = results;
        self.batch_position = 0;

        Ok(())
    }
}

#[pymethods]
impl Cursor {
    /// Get the next document (lazy loading)
    fn next<'py>(&mut self, py: Python<'py>) -> PyResult<PyObject> {
        // If current batch is exhausted, fetch next batch
        if self.batch_position >= self.current_batch.len() {
            if self.exhausted {
                return Ok(py.None());
            }
            self.fetch_next_batch()?;
            if self.current_batch.is_empty() {
                return Ok(py.None());
            }
        }

        let doc = &self.current_batch[self.batch_position];
        self.batch_position += 1;

        let py_dict = json_to_python_dict(py, doc)?;
        Ok(py_dict.into_any().unbind())
    }

    /// Get the next batch of documents
    fn next_batch<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        if self.exhausted && self.batch_position >= self.current_batch.len() {
            return Ok(PyList::empty(py));
        }

        // Fetch a fresh batch from DB
        self.fetch_next_batch()?;

        // Convert entire batch to Python list
        let py_list = PyList::empty(py);
        for doc in &self.current_batch {
            let py_dict = json_to_python_dict(py, doc)?;
            py_list.append(py_dict)?;
        }
        self.batch_position = self.current_batch.len(); // Mark as consumed

        Ok(py_list)
    }

    /// Get next chunk of N documents
    fn next_chunk<'py>(
        &mut self,
        py: Python<'py>,
        chunk_size: usize,
    ) -> PyResult<Bound<'py, PyList>> {
        let py_list = PyList::empty(py);
        let mut count = 0;

        while count < chunk_size {
            // Refill buffer if needed
            if self.batch_position >= self.current_batch.len() {
                if self.exhausted {
                    break;
                }
                self.fetch_next_batch()?;
                if self.current_batch.is_empty() {
                    break;
                }
            }

            let doc = &self.current_batch[self.batch_position];
            self.batch_position += 1;
            count += 1;

            let py_dict = json_to_python_dict(py, doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Get current position (total documents read so far)
    fn position(&self) -> usize {
        // position tracks where we are in the DB, batch_position is local
        self.position - self.current_batch.len() + self.batch_position
    }

    /// Check if cursor is exhausted
    fn is_finished(&self) -> bool {
        self.exhausted && self.batch_position >= self.current_batch.len()
    }

    /// Reset cursor to beginning
    fn rewind(&mut self) {
        self.position = 0;
        self.batch_position = 0;
        self.current_batch.clear();
        self.exhausted = false;
    }

    /// Skip N documents
    fn skip(&mut self, n: usize) {
        // Advance position - next fetch will skip these
        self.position += n;
        self.current_batch.clear();
        self.batch_position = 0;
    }

    /// Take up to N documents
    fn take<'py>(&mut self, py: Python<'py>, n: usize) -> PyResult<Bound<'py, PyList>> {
        self.next_chunk(py, n)
    }

    /// Collect all remaining documents (use with caution on large datasets!)
    fn collect_all<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let py_list = PyList::empty(py);

        loop {
            // Refill buffer if needed
            if self.batch_position >= self.current_batch.len() {
                if self.exhausted {
                    break;
                }
                self.fetch_next_batch()?;
                if self.current_batch.is_empty() {
                    break;
                }
            }

            while self.batch_position < self.current_batch.len() {
                let doc = &self.current_batch[self.batch_position];
                self.batch_position += 1;
                let py_dict = json_to_python_dict(py, doc)?;
                py_list.append(py_dict)?;
            }
        }

        Ok(py_list)
    }

    /// Python iterator protocol
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Get next for Python iteration (lazy loading)
    fn __next__<'py>(&mut self, py: Python<'py>) -> PyResult<Option<PyObject>> {
        // Refill buffer if needed
        if self.batch_position >= self.current_batch.len() {
            if self.exhausted {
                return Ok(None);
            }
            self.fetch_next_batch()?;
            if self.current_batch.is_empty() {
                return Ok(None);
            }
        }

        let doc = &self.current_batch[self.batch_position];
        self.batch_position += 1;

        let py_dict = json_to_python_dict(py, doc)?;
        Ok(Some(py_dict.into_any().unbind()))
    }

    fn __repr__(&self) -> String {
        format!(
            "Cursor(position={}, batch_size={}, exhausted={})",
            self.position(),
            self.batch_size,
            self.is_finished()
        )
    }
}

// ========== HELPER FUNCTIONS ==========

/// Convert DocumentId to Python value
fn doc_id_to_py(py: Python<'_>, id: &DocumentId) -> PyResult<PyObject> {
    match id {
        DocumentId::Int(i) => Ok(i.into_pyobject(py)?.into_any().unbind()),
        DocumentId::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        DocumentId::ObjectId(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
    }
}

/// Python value -> JSON
#[allow(clippy::only_used_in_recursion)]
fn python_to_json(py: Python<'_>, value: &Bound<'_, pyo3::PyAny>) -> PyResult<Value> {
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
            arr.push(python_to_json(py, &item)?);
        }
        Ok(Value::Array(arr))
    } else if let Ok(dict) = value.downcast::<PyDict>() {
        let mut map = serde_json::Map::new();
        for (k, v) in dict.iter() {
            let key: String = k.extract()?;
            map.insert(key, python_to_json(py, &v)?);
        }
        Ok(Value::Object(map))
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
            "Unsupported type: {:?}",
            value.get_type()
        )))
    }
}

/// Python dict -> JSON Value
fn python_dict_to_json_value(py: Python<'_>, dict: &Bound<'_, PyDict>) -> PyResult<Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in dict.iter() {
        let key: String = k.extract()?;
        map.insert(key, python_to_json(py, &v)?);
    }
    Ok(Value::Object(map))
}

/// JSON Value -> Python dict
fn json_to_python_dict<'py>(py: Python<'py>, value: &Value) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);

    if let Value::Object(map) = value {
        for (key, val) in map.iter() {
            let py_val = json_value_to_python(py, val)?;
            dict.set_item(key, py_val)?;
        }
    }

    Ok(dict)
}

/// JSON Value -> Python value
fn json_value_to_python(py: Python<'_>, value: &Value) -> PyResult<PyObject> {
    match value {
        Value::Null => Ok(py.None()),
        Value::Bool(b) => Ok(b.into_pyobject(py)?.to_owned().into_any().unbind()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.into_pyobject(py)?.into_any().unbind())
            } else if let Some(f) = n.as_f64() {
                Ok(f.into_pyobject(py)?.into_any().unbind())
            } else {
                Ok(py.None())
            }
        }
        Value::String(s) => Ok(s.into_pyobject(py)?.into_any().unbind()),
        Value::Array(arr) => {
            let py_list = PyList::empty(py);
            for item in arr {
                py_list.append(json_value_to_python(py, item)?)?;
            }
            Ok(py_list.into_any().unbind())
        }
        Value::Object(map) => {
            let py_dict = PyDict::new(py);
            for (k, v) in map.iter() {
                py_dict.set_item(k, json_value_to_python(py, v)?)?;
            }
            Ok(py_dict.into_any().unbind())
        }
    }
}

/// Python module initialization
#[pymodule]
fn ironbase(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<IronBase>()?;
    m.add_class::<Collection>()?;
    m.add_class::<Cursor>()?;
    Ok(())
}
