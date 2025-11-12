// bindings/python/src/lib.rs
// PyO3 wrapper for ironbase-core

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
// Arc and RwLock are used internally by DatabaseCore/CollectionCore
use serde_json::Value;
use std::collections::HashMap;

use ironbase_core::{DatabaseCore, CollectionCore, DocumentId, InsertManyResult};

/// IronBase Database - Python wrapper
#[pyclass]
pub struct IronBase {
    db: DatabaseCore,
}

#[pymethods]
impl IronBase {
    /// Új adatbázis megnyitása vagy létrehozása
    #[new]
    fn new(path: String) -> PyResult<Self> {
        let db = DatabaseCore::open(&path)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        Ok(IronBase { db })
    }

    /// Collection lekérése (ha nem létezik, létrehozza)
    fn collection(&self, name: String) -> PyResult<Collection> {
        let coll_core = self.db.collection(&name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Ok(Collection { core: coll_core })
    }

    /// Collection-ök listája
    fn list_collections(&self) -> PyResult<Vec<String>> {
        Ok(self.db.list_collections())
    }

    /// Collection törlése
    fn drop_collection(&self, name: String) -> PyResult<()> {
        self.db.drop_collection(&name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Adatbázis bezárása és flush
    fn close(&self) -> PyResult<()> {
        self.db.flush()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))
    }

    /// Adatbázis statisztikák
    fn stats(&self) -> PyResult<String> {
        Ok(serde_json::to_string_pretty(&self.db.stats()).unwrap())
    }

    /// Set the global log level
    ///
    /// Args:
    ///     level (str): One of "ERROR", "WARN", "INFO", "DEBUG", "TRACE"
    ///
    /// Example:
    ///     db.set_log_level("DEBUG")  # Show debug messages
    ///     db.set_log_level("WARN")   # Only warnings and errors (default)
    ///     db.set_log_level("TRACE")  # Show everything (very verbose)
    #[staticmethod]
    fn set_log_level(level: String) -> PyResult<()> {
        let log_level = ironbase_core::LogLevel::from_str(&level)
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid log level '{}'. Must be one of: ERROR, WARN, INFO, DEBUG, TRACE", level)
            ))?;

        ironbase_core::set_log_level(log_level);
        Ok(())
    }

    /// Get the current global log level
    ///
    /// Returns:
    ///     str: Current log level ("ERROR", "WARN", "INFO", "DEBUG", or "TRACE")
    #[staticmethod]
    fn get_log_level() -> PyResult<String> {
        let level = ironbase_core::get_log_level();
        Ok(level.as_str().to_string())
    }

    /// Storage compaction - removes tombstones and old document versions
    /// Returns compaction statistics as a dict
    fn compact(&self) -> PyResult<PyObject> {
        let stats = self.db.compact()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Python::with_gil(|py| {
            let dict = PyDict::new(py);
            dict.set_item("size_before", stats.size_before)?;
            dict.set_item("size_after", stats.size_after)?;
            dict.set_item("space_saved", stats.space_saved())?;
            dict.set_item("documents_scanned", stats.documents_scanned)?;
            dict.set_item("documents_kept", stats.documents_kept)?;
            dict.set_item("tombstones_removed", stats.tombstones_removed)?;
            dict.set_item("peak_memory_mb", stats.peak_memory_mb)?;
            dict.set_item("compression_ratio", stats.compression_ratio())?;
            Ok(dict.into())
        })
    }

    fn __repr__(&self) -> String {
        format!("IronBase('{}')", self.db.path())
    }

    // ========== ACD TRANSACTION API ==========

    /// Begin a new transaction
    /// Returns the transaction ID
    fn begin_transaction(&self) -> PyResult<u64> {
        Ok(self.db.begin_transaction())
    }

    /// Commit a transaction (applies all buffered operations atomically)
    fn commit_transaction(&self, tx_id: u64) -> PyResult<()> {
        self.db.commit_transaction(tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Rollback a transaction (discard all buffered operations)
    fn rollback_transaction(&self, tx_id: u64) -> PyResult<()> {
        self.db.rollback_transaction(tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    // ========== COLLECTION TRANSACTION METHODS ==========

    /// Insert one document within a transaction
    ///
    /// Args:
    ///     collection_name: str - Name of the collection
    ///     document: dict - Document to insert
    ///     tx_id: int - Transaction ID from begin_transaction()
    ///
    /// Returns:
    ///     dict - {"acknowledged": True, "inserted_id": <id>}
    ///
    /// Example:
    ///     tx_id = db.begin_transaction()
    ///     db.insert_one_tx("users", {"name": "Alice"}, tx_id)
    ///     db.commit_transaction(tx_id)
    fn insert_one_tx(&self, collection_name: String, document: &PyDict, tx_id: u64) -> PyResult<PyObject> {
        // Convert Python dict to HashMap
        let mut doc_map: HashMap<String, Value> = HashMap::new();
        for (key, value) in document.iter() {
            let key_str: String = key.extract()?;
            let json_value = python_to_json(value)?;
            doc_map.insert(key_str, json_value);
        }

        // Call Rust core (ALL logic in core)
        let inserted_id = self.db.insert_one_tx(&collection_name, doc_map, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Return result
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

    /// Update one document within a transaction
    ///
    /// Args:
    ///     collection_name: str - Name of the collection
    ///     query: dict - Query to match document
    ///     new_doc: dict - New document content (not update operators)
    ///     tx_id: int - Transaction ID from begin_transaction()
    ///
    /// Returns:
    ///     dict - {"acknowledged": True, "matched_count": <n>, "modified_count": <n>}
    ///
    /// Example:
    ///     tx_id = db.begin_transaction()
    ///     db.update_one_tx("users", {"name": "Alice"}, {"name": "Alice", "age": 30}, tx_id)
    ///     db.commit_transaction(tx_id)
    fn update_one_tx(&self, collection_name: String, query: &PyDict, new_doc: &PyDict, tx_id: u64) -> PyResult<PyObject> {
        // Convert Python dicts to JSON
        let query_json = python_dict_to_json_value(query)?;
        let new_doc_json = python_dict_to_json_value(new_doc)?;

        // Call Rust core (ALL logic in core)
        let (matched_count, modified_count) = self.db.update_one_tx(&collection_name, &query_json, new_doc_json, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("matched_count", matched_count)?;
            result.set_item("modified_count", modified_count)?;
            Ok(result.into())
        })
    }

    /// Delete one document within a transaction
    ///
    /// Args:
    ///     collection_name: str - Name of the collection
    ///     query: dict - Query to match document
    ///     tx_id: int - Transaction ID from begin_transaction()
    ///
    /// Returns:
    ///     dict - {"acknowledged": True, "deleted_count": <n>}
    ///
    /// Example:
    ///     tx_id = db.begin_transaction()
    ///     db.delete_one_tx("users", {"name": "Alice"}, tx_id)
    ///     db.commit_transaction(tx_id)
    fn delete_one_tx(&self, collection_name: String, query: &PyDict, tx_id: u64) -> PyResult<PyObject> {
        // Convert Python dict to JSON
        let query_json = python_dict_to_json_value(query)?;

        // Call Rust core (ALL logic in core)
        let deleted_count = self.db.delete_one_tx(&collection_name, &query_json, tx_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Return result
        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", deleted_count)?;
            Ok(result.into())
        })
    }
}

/// Collection - Python wrapper for CollectionCore
#[pyclass]
pub struct Collection {
    core: CollectionCore,
}

#[pymethods]
impl Collection {
    /// Insert one document
    fn insert_one(&self, document: &PyDict) -> PyResult<PyObject> {
        let mut doc_map: HashMap<String, Value> = HashMap::new();

        // Python dict -> HashMap konverzió
        for (key, value) in document.iter() {
            let key_str: String = key.extract()?;
            let json_value = python_to_json(value)?;
            doc_map.insert(key_str, json_value);
        }

        // Call core method
        let inserted_id = self.core.insert_one(doc_map)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

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

    /// Insert many documents - optimized batch insert
    fn insert_many(&self, documents: &PyList) -> PyResult<PyObject> {
        // Convert Python list to Vec<HashMap>
        let mut docs = Vec::with_capacity(documents.len());
        for doc in documents.iter() {
            let doc_dict: &PyDict = doc.downcast()?;
            let mut fields = HashMap::new();

            for (key, value) in doc_dict.iter() {
                let key_str: String = key.extract()?;
                let value_json = python_to_json(value)?;
                fields.insert(key_str, value_json);
            }

            docs.push(fields);
        }

        // Call Rust core insert_many (ALL logic in core)
        let result = self.core.insert_many(docs)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Convert result back to Python
        Python::with_gil(|py| {
            let result_dict = PyDict::new(py);
            result_dict.set_item("acknowledged", true)?;
            result_dict.set_item("inserted_count", result.inserted_count)?;

            // Convert inserted_ids to Python list
            let ids_list = PyList::empty(py);
            for doc_id in result.inserted_ids {
                match doc_id {
                    DocumentId::Int(i) => ids_list.append(i)?,
                    DocumentId::String(s) => ids_list.append(s)?,
                    DocumentId::ObjectId(oid) => ids_list.append(oid)?,
                }
            }
            result_dict.set_item("inserted_ids", ids_list)?;

            Ok(result_dict.into())
        })
    }

    /// Find documents with optional projection, sort, limit, skip
    #[pyo3(signature = (query=None, projection=None, sort=None, limit=None, skip=None))]
    fn find(
        &self,
        query: Option<&PyDict>,
        projection: Option<&PyDict>,
        sort: Option<&PyList>,
        limit: Option<usize>,
        skip: Option<usize>,
    ) -> PyResult<PyObject> {
        use ironbase_core::find_options::FindOptions;
        use std::collections::HashMap;

        // Parse query (empty query = all documents)
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        // Build FindOptions
        let mut options = FindOptions::new();

        // Convert projection
        if let Some(proj) = projection {
            let mut projection_map = HashMap::new();
            for (key, value) in proj.iter() {
                let field: String = key.extract()?;
                let action: i32 = value.extract()?;
                projection_map.insert(field, action);
            }
            options.projection = Some(projection_map);
        }

        // Convert sort
        if let Some(sort_list) = sort {
            let mut sort_vec = Vec::new();
            for item in sort_list.iter() {
                let tuple: &PyTuple = item.downcast()?;
                let field: String = tuple.get_item(0)?.extract()?;
                let direction: i32 = tuple.get_item(1)?.extract()?;
                sort_vec.push((field, direction));
            }
            options.sort = Some(sort_vec);
        }

        // Set limit and skip
        options.limit = limit;
        options.skip = skip;

        // Call core method
        let results = self.core.find_with_options(&query_json, options)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

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

    /// Find one document
    fn find_one(&self, query: Option<&PyDict>) -> PyResult<PyObject> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        // Call core method
        let result = self.core.find_one(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Convert to Python
        Python::with_gil(|py| {
            match result {
                Some(doc) => {
                    let py_dict = json_to_python_dict(py, &doc)?;
                    Ok(py_dict.into())
                }
                None => Ok(py.None()),
            }
        })
    }

    /// Count documents
    fn count_documents(&self, query: Option<&PyDict>) -> PyResult<u64> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        self.core.count_documents(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Distinct values
    fn distinct(&self, field: &str, query: Option<&PyDict>) -> PyResult<PyObject> {
        let query_json = match query {
            Some(q) => python_dict_to_json_value(q)?,
            None => serde_json::json!({}),
        };

        let distinct_values = self.core.distinct(field, &query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

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

    /// Update one document
    fn update_one(&self, query: &PyDict, update: &PyDict) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;
        let update_json = python_dict_to_json_value(update)?;

        let (matched_count, modified_count) = self.core.update_one(&query_json, &update_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("matched_count", matched_count)?;
            result.set_item("modified_count", modified_count)?;
            Ok(result.into())
        })
    }

    /// Update many documents
    fn update_many(&self, query: &PyDict, update: &PyDict) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;
        let update_json = python_dict_to_json_value(update)?;

        let (matched_count, modified_count) = self.core.update_many(&query_json, &update_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("matched_count", matched_count)?;
            result.set_item("modified_count", modified_count)?;
            Ok(result.into())
        })
    }

    /// Delete one document
    fn delete_one(&self, query: &PyDict) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;

        let deleted_count = self.core.delete_one(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", deleted_count)?;
            Ok(result.into())
        })
    }

    /// Delete many documents
    fn delete_many(&self, query: &PyDict) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;

        let deleted_count = self.core.delete_many(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        Python::with_gil(|py| {
            let result = PyDict::new(py);
            result.set_item("acknowledged", true)?;
            result.set_item("deleted_count", deleted_count)?;
            Ok(result.into())
        })
    }

    /// Create an index on a field
    ///
    /// Args:
    ///     field: str - Field name to index
    ///     unique: bool - Whether the index should enforce uniqueness (default: False)
    ///
    /// Returns:
    ///     str - Index name
    ///
    /// Example:
    ///     collection.create_index("email", unique=True)
    ///     collection.create_index("age")
    #[pyo3(signature = (field, unique=false))]
    fn create_index(&self, field: String, unique: bool) -> PyResult<String> {
        self.core.create_index(field, unique)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// Drop an index
    ///
    /// Args:
    ///     index_name: str - Name of the index to drop
    ///
    /// Example:
    ///     collection.drop_index("users_email")
    fn drop_index(&self, index_name: String) -> PyResult<()> {
        self.core.drop_index(&index_name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }

    /// List all indexes in this collection
    ///
    /// Returns:
    ///     list - List of index names
    ///
    /// Example:
    ///     indexes = collection.list_indexes()
    ///     print(indexes)  # ['users_id', 'users_email', 'users_age']
    fn list_indexes(&self) -> PyResult<Vec<String>> {
        Ok(self.core.list_indexes())
    }

    /// Explain the query execution plan without executing the query
    ///
    /// Args:
    ///     query: dict - MongoDB-style query
    ///
    /// Returns:
    ///     dict - Query plan with information about index usage
    ///
    /// Example:
    ///     plan = collection.explain({"age": 25})
    ///     print(plan["queryPlan"])  # "IndexScan" or "CollectionScan"
    ///     print(plan["indexUsed"])  # "users_age" or null
    fn explain(&self, query: &PyDict) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;

        let plan = self.core.explain(&query_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

        // Convert JSON Value to Python dict
        Python::with_gil(|py| {
            let py_dict = json_to_python_dict(py, &plan)?;
            Ok(py_dict.into())
        })
    }

    /// Execute a query with manual index selection (hint)
    ///
    /// Args:
    ///     query: dict - MongoDB-style query
    ///     hint: str - Index name to use
    ///
    /// Returns:
    ///     list - Matching documents
    ///
    /// Example:
    ///     # Force use of age index even if planner would choose differently
    ///     results = collection.find_with_hint({"age": 25}, "users_age")
    fn find_with_hint(&self, query: &PyDict, hint: String) -> PyResult<PyObject> {
        let query_json = python_dict_to_json_value(query)?;

        let results = self.core.find_with_hint(&query_json, &hint)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

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

    /// Execute aggregation pipeline
    ///
    /// Args:
    ///     pipeline: list - List of aggregation stage dictionaries
    ///
    /// Returns:
    ///     list - Aggregation results
    ///
    /// Example:
    ///     # Group users by city and count
    ///     results = collection.aggregate([
    ///         {"$match": {"age": {"$gte": 18}}},
    ///         {"$group": {"_id": "$city", "count": {"$sum": 1}}},
    ///         {"$sort": {"count": -1}}
    ///     ])
    fn aggregate(&self, pipeline: &PyList) -> PyResult<PyObject> {
        // Convert Python list to JSON array
        let mut stages = Vec::new();
        for stage in pipeline.iter() {
            let stage_dict: &PyDict = stage.downcast()?;
            let stage_json = python_dict_to_json_value(stage_dict)?;
            stages.push(stage_json);
        }

        let pipeline_json = serde_json::Value::Array(stages);

        // Execute aggregation
        let results = self.core.aggregate(&pipeline_json)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))?;

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

    fn __repr__(&self) -> String {
        format!("Collection('{}')", self.core.name)
    }
}

// ========== PYTHON <-> JSON CONVERSION HELPERS ==========

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

/// Python modul inicializálás
#[pymodule]
fn ironbase(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<IronBase>()?;
    m.add_class::<Collection>()?;
    Ok(())
}
