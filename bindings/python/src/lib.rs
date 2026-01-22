// bindings/python/src/lib.rs
// PyO3 0.24 wrapper for ironbase-core

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use ironbase_core::{
    index::FuzzyAlgorithm, storage::MemoryStorage, CollectionCore, DatabaseCore, DocumentId,
    DurabilityMode, IronBaseError, StorageEngine,
};

const DEFAULT_FIND_LIMIT: usize = 10_000;

// ========== CUSTOM PYTHON EXCEPTIONS ==========
// These allow Python code to catch specific error types:
//   try:
//       coll.find(...)
//   except ironbase.CollectionNotFoundError:
//       ...
//   except ironbase.OutOfMemoryError:
//       ...

pyo3::create_exception!(ironbase, IronBaseException, pyo3::exceptions::PyException);
pyo3::create_exception!(ironbase, CollectionNotFoundError, IronBaseException);
pyo3::create_exception!(ironbase, CollectionExistsError, IronBaseException);
pyo3::create_exception!(ironbase, DocumentNotFoundError, IronBaseException);
pyo3::create_exception!(ironbase, InvalidQueryError, IronBaseException);
pyo3::create_exception!(ironbase, CorruptionError, IronBaseException);
pyo3::create_exception!(ironbase, IndexError, IronBaseException);
pyo3::create_exception!(ironbase, AggregationError, IronBaseException);
pyo3::create_exception!(ironbase, SchemaValidationError, IronBaseException);
pyo3::create_exception!(ironbase, TransactionError, IronBaseException);
pyo3::create_exception!(ironbase, DatabaseLockedError, IronBaseException);
pyo3::create_exception!(ironbase, DatabaseClosedError, IronBaseException);
pyo3::create_exception!(ironbase, OperationNotAllowedError, IronBaseException);
pyo3::create_exception!(ironbase, OutOfMemoryError, IronBaseException);
pyo3::create_exception!(ironbase, CancelledError, IronBaseException);
pyo3::create_exception!(ironbase, TimeoutError, IronBaseException);
pyo3::create_exception!(ironbase, SerializationError, IronBaseException);

/// Convert IronBaseError to appropriate Python exception
fn ironbase_error_to_pyerr(e: IronBaseError) -> PyErr {
    match e {
        IronBaseError::Io(ref _inner) => {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string())
        }
        IronBaseError::Serialization(_) => PyErr::new::<SerializationError, _>(e.to_string()),
        IronBaseError::Deserialization(_) => PyErr::new::<SerializationError, _>(e.to_string()),
        IronBaseError::CollectionNotFound(_) => {
            PyErr::new::<CollectionNotFoundError, _>(e.to_string())
        }
        IronBaseError::CollectionExists(_) => PyErr::new::<CollectionExistsError, _>(e.to_string()),
        IronBaseError::DocumentNotFound => PyErr::new::<DocumentNotFoundError, _>(e.to_string()),
        IronBaseError::InvalidQuery(_) => PyErr::new::<InvalidQueryError, _>(e.to_string()),
        IronBaseError::Corruption(_) => PyErr::new::<CorruptionError, _>(e.to_string()),
        IronBaseError::IndexError(_) => PyErr::new::<IndexError, _>(e.to_string()),
        IronBaseError::AggregationError(_) => PyErr::new::<AggregationError, _>(e.to_string()),
        IronBaseError::SchemaError(_) => PyErr::new::<SchemaValidationError, _>(e.to_string()),
        IronBaseError::TransactionCommitted => PyErr::new::<TransactionError, _>(e.to_string()),
        IronBaseError::TransactionAborted(_) => PyErr::new::<TransactionError, _>(e.to_string()),
        IronBaseError::WALCorruption => PyErr::new::<CorruptionError, _>(e.to_string()),
        IronBaseError::DatabaseLocked(_) => PyErr::new::<DatabaseLockedError, _>(e.to_string()),
        IronBaseError::DatabaseClosed => PyErr::new::<DatabaseClosedError, _>(e.to_string()),
        IronBaseError::OperationNotAllowed(_) => {
            PyErr::new::<OperationNotAllowedError, _>(e.to_string())
        }
        IronBaseError::Unknown(_) => {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string())
        }
        IronBaseError::OutOfMemory(_) => PyErr::new::<OutOfMemoryError, _>(e.to_string()),
        IronBaseError::Cancelled(_) => PyErr::new::<CancelledError, _>(e.to_string()),
        IronBaseError::Timeout(_) => PyErr::new::<TimeoutError, _>(e.to_string()),
    }
}

/// Database wrapper enum to support both file and memory storage
#[derive(Clone)]
enum DatabaseWrapper {
    File(Arc<DatabaseCore<StorageEngine>>),
    Memory(Arc<DatabaseCore<MemoryStorage>>),
}

/// Collection wrapper enum
enum CollectionWrapper {
    File(CollectionCore<StorageEngine>),
    Memory(CollectionCore<MemoryStorage>),
}

// Macro to reduce boilerplate for DatabaseWrapper methods
macro_rules! db_dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            DatabaseWrapper::File(db) => db.$method($($arg),*),
            DatabaseWrapper::Memory(db) => db.$method($($arg),*),
        }
    };
}

// Macro to reduce boilerplate for CollectionWrapper methods
macro_rules! coll_dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            CollectionWrapper::File(c) => c.$method($($arg),*),
            CollectionWrapper::Memory(c) => c.$method($($arg),*),
        }
    };
}

impl DatabaseWrapper {
    fn list_collections(&self) -> Vec<String> {
        db_dispatch!(self, list_collections)
    }

    fn path(&self) -> String {
        match self {
            DatabaseWrapper::File(db) => db.path().to_string(),
            DatabaseWrapper::Memory(_) => ":memory:".to_string(),
        }
    }

    fn collection(&self, name: &str) -> Result<CollectionWrapper, ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.collection(name).map(CollectionWrapper::File),
            DatabaseWrapper::Memory(db) => db.collection(name).map(CollectionWrapper::Memory),
        }
    }

    fn set_collection_schema(
        &self,
        name: &str,
        schema: Option<Value>,
    ) -> Result<(), ironbase_core::IronBaseError> {
        db_dispatch!(self, set_collection_schema, name, schema)
    }

    fn drop_collection(&self, name: &str) -> Result<(), ironbase_core::IronBaseError> {
        db_dispatch!(self, drop_collection, name)
    }

    fn close(&self) -> Result<(), ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.close(),
            DatabaseWrapper::Memory(_) => Ok(()), // No-op for memory storage
        }
    }

    fn checkpoint(&self) -> Result<(), ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => {
                db.checkpoint()?;
                Ok(())
            }
            DatabaseWrapper::Memory(_) => Ok(()), // No-op for memory storage
        }
    }

    fn stats(&self) -> Value {
        match self {
            DatabaseWrapper::File(db) => db.stats(),
            DatabaseWrapper::Memory(db) => {
                // Basic stats for memory storage
                serde_json::json!({
                    "storage": "memory",
                    "collections": db.list_collections()
                })
            }
        }
    }

    fn compact(&self) -> Result<ironbase_core::CompactionStats, ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.compact(),
            DatabaseWrapper::Memory(_) => Ok(ironbase_core::CompactionStats::default()),
        }
    }

    fn begin_transaction(&self) -> u64 {
        db_dispatch!(self, begin_transaction)
    }

    fn commit_transaction(&self, tx_id: u64) -> Result<(), ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.commit_transaction(tx_id),
            DatabaseWrapper::Memory(_) => Err(ironbase_core::IronBaseError::OperationNotAllowed(
                "Transactions are not supported for in-memory databases".to_string(),
            )),
        }
    }

    fn rollback_transaction(&self, tx_id: u64) -> Result<(), ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.rollback_transaction(tx_id),
            DatabaseWrapper::Memory(_) => Err(ironbase_core::IronBaseError::OperationNotAllowed(
                "Transactions are not supported for in-memory databases".to_string(),
            )),
        }
    }

    fn insert_one(
        &self,
        collection: &str,
        doc: HashMap<String, Value>,
    ) -> Result<DocumentId, ironbase_core::IronBaseError> {
        db_dispatch!(self, insert_one, collection, doc)
    }

    fn insert_many(
        &self,
        collection: &str,
        docs: Vec<HashMap<String, Value>>,
    ) -> Result<Vec<DocumentId>, ironbase_core::IronBaseError> {
        db_dispatch!(self, insert_many, collection, docs)
    }

    #[allow(dead_code)] // Kept for backward compatibility
    fn update_one(
        &self,
        collection: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64), ironbase_core::IronBaseError> {
        db_dispatch!(self, update_one, collection, query, update)
    }

    fn update_one_with_options(
        &self,
        collection: &str,
        query: &Value,
        update: &Value,
        options: ironbase_core::UpdateOptions,
    ) -> Result<ironbase_core::UpdateResult, ironbase_core::IronBaseError> {
        db_dispatch!(
            self,
            update_one_with_options,
            collection,
            query,
            update,
            options
        )
    }

    fn update_many(
        &self,
        collection: &str,
        query: &Value,
        update: &Value,
    ) -> Result<(u64, u64), ironbase_core::IronBaseError> {
        db_dispatch!(self, update_many, collection, query, update)
    }

    fn delete_one(
        &self,
        collection: &str,
        query: &Value,
    ) -> Result<u64, ironbase_core::IronBaseError> {
        db_dispatch!(self, delete_one, collection, query)
    }

    fn delete_many(
        &self,
        collection: &str,
        query: &Value,
    ) -> Result<u64, ironbase_core::IronBaseError> {
        db_dispatch!(self, delete_many, collection, query)
    }

    fn insert_one_tx(
        &self,
        collection: &str,
        doc: HashMap<String, Value>,
        tx_id: u64,
    ) -> Result<DocumentId, ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.insert_one_tx(collection, doc, tx_id),
            DatabaseWrapper::Memory(_) => Err(ironbase_core::IronBaseError::OperationNotAllowed(
                "Transactions are not supported for in-memory databases".to_string(),
            )),
        }
    }

    fn update_one_tx(
        &self,
        collection: &str,
        query: &Value,
        update: Value,
        tx_id: u64,
    ) -> Result<(u64, u64), ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.update_one_tx(collection, query, update, tx_id),
            DatabaseWrapper::Memory(_) => Err(ironbase_core::IronBaseError::OperationNotAllowed(
                "Transactions are not supported for in-memory databases".to_string(),
            )),
        }
    }

    fn delete_one_tx(
        &self,
        collection: &str,
        query: &Value,
        tx_id: u64,
    ) -> Result<u64, ironbase_core::IronBaseError> {
        match self {
            DatabaseWrapper::File(db) => db.delete_one_tx(collection, query, tx_id),
            DatabaseWrapper::Memory(_) => Err(ironbase_core::IronBaseError::OperationNotAllowed(
                "Transactions are not supported for in-memory databases".to_string(),
            )),
        }
    }
}

impl CollectionWrapper {
    fn name(&self) -> &str {
        match self {
            CollectionWrapper::File(c) => &c.name,
            CollectionWrapper::Memory(c) => &c.name,
        }
    }

    fn set_schema(&self, schema: Option<Value>) -> Result<(), ironbase_core::IronBaseError> {
        coll_dispatch!(self, set_schema, schema)
    }

    fn get_schema(&self) -> Result<Option<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, get_schema)
    }

    fn find_with_options(
        &self,
        query: &Value,
        options: ironbase_core::FindOptions,
    ) -> Result<Vec<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, find_with_options, query, options)
    }

    fn find_with_result(
        &self,
        query: &Value,
        options: ironbase_core::FindOptions,
    ) -> Result<ironbase_core::FindResult, ironbase_core::IronBaseError> {
        coll_dispatch!(self, find_with_result, query, options)
    }

    fn count_documents(&self, query: &Value) -> Result<u64, ironbase_core::IronBaseError> {
        coll_dispatch!(self, count_documents, query)
    }

    fn distinct(
        &self,
        field: &str,
        query: &Value,
    ) -> Result<Vec<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, distinct, field, query)
    }

    fn create_index(
        &self,
        field: String,
        unique: bool,
        sparse: bool,
    ) -> Result<String, ironbase_core::IronBaseError> {
        coll_dispatch!(self, create_index, field, unique, sparse)
    }

    fn create_compound_index(
        &self,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<String, ironbase_core::IronBaseError> {
        coll_dispatch!(self, create_compound_index, fields, unique, sparse)
    }

    fn drop_index(&self, name: &str) -> Result<(), ironbase_core::IronBaseError> {
        coll_dispatch!(self, drop_index, name)
    }

    fn list_indexes(&self) -> Result<Vec<String>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, list_indexes)
    }

    fn create_fuzzy_index(
        &self,
        field: String,
        algo: FuzzyAlgorithm,
        threshold: f64,
    ) -> Result<String, ironbase_core::IronBaseError> {
        coll_dispatch!(self, create_fuzzy_index, field, algo, threshold)
    }

    fn fuzzy_search(
        &self,
        field: &str,
        query: &str,
        threshold: Option<f64>,
        algo: Option<FuzzyAlgorithm>,
    ) -> Result<Vec<(Value, f64)>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, fuzzy_search, field, query, threshold, algo)
    }

    fn create_fulltext_index(
        &self,
        field: String,
        language: &str,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> Result<String, ironbase_core::IronBaseError> {
        coll_dispatch!(
            self,
            create_fulltext_index,
            field,
            language,
            min_word_length,
            accent_folding
        )
    }

    fn fulltext_search(
        &self,
        field: &str,
        query: &str,
        limit: Option<usize>,
        skip: Option<usize>,
        min_score: Option<f64>,
        projection: Option<HashMap<String, i32>>,
    ) -> Result<Vec<(Value, f64, Vec<String>)>, ironbase_core::IronBaseError> {
        coll_dispatch!(
            self,
            fulltext_search,
            field,
            query,
            limit,
            skip,
            min_score,
            projection
        )
    }

    fn list_fulltext_indexes(
        &self,
    ) -> Result<Vec<ironbase_core::fulltext::FulltextIndexMetadata>, ironbase_core::IronBaseError>
    {
        coll_dispatch!(self, list_fulltext_indexes)
    }

    fn explain(&self, query: &Value) -> Result<Value, ironbase_core::IronBaseError> {
        coll_dispatch!(self, explain, query)
    }

    fn find_with_hint(
        &self,
        query: &Value,
        hint: &str,
    ) -> Result<Vec<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, find_with_hint, query, hint)
    }

    fn aggregate(&self, pipeline: &Value) -> Result<Vec<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, aggregate, pipeline)
    }

    fn aggregate_auto(&self, pipeline: &Value) -> Result<Vec<Value>, ironbase_core::IronBaseError> {
        coll_dispatch!(self, aggregate_auto, pipeline)
    }
}

/// IronBase Database - Python wrapper
#[pyclass]
pub struct IronBase {
    db: DatabaseWrapper,
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

        let db =
            DatabaseCore::open_with_durability(&path, mode).map_err(ironbase_error_to_pyerr)?;

        Ok(IronBase {
            db: DatabaseWrapper::File(Arc::new(db)),
        })
    }

    /// Create an in-memory database (10-100x faster, no persistence)
    ///
    /// Use this for testing and temporary data. Data is lost when the database
    /// is closed or when the Python process exits.
    ///
    /// Example:
    ///     >>> db = IronBase.open_memory()
    ///     >>> coll = db.collection("test")
    ///     >>> coll.insert_one({"name": "Alice"})
    #[staticmethod]
    fn open_memory() -> PyResult<Self> {
        let db = DatabaseCore::<MemoryStorage>::open_memory().map_err(ironbase_error_to_pyerr)?;

        Ok(IronBase {
            db: DatabaseWrapper::Memory(Arc::new(db)),
        })
    }

    /// Get or create a collection
    fn collection(&self, name: String) -> PyResult<Collection> {
        let coll_wrapper = self.db.collection(&name).map_err(ironbase_error_to_pyerr)?;

        Ok(Collection {
            core: coll_wrapper,
            db: self.db.clone(),
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
            .map_err(ironbase_error_to_pyerr)
    }

    /// Drop a collection
    fn drop_collection(&self, name: String) -> PyResult<()> {
        self.db
            .drop_collection(&name)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Close the database: flush all changes and release the file lock.
    ///
    /// After calling close(), another process can open the same database file.
    /// The database instance should not be used after calling close().
    fn close(&self) -> PyResult<()> {
        self.db.close().map_err(ironbase_error_to_pyerr)
    }

    /// Checkpoint - Clear WAL
    fn checkpoint(&self) -> PyResult<()> {
        self.db.checkpoint().map_err(ironbase_error_to_pyerr)
    }

    /// Get database statistics
    fn stats(&self) -> PyResult<String> {
        serde_json::to_string_pretty(&self.db.stats())
            .map_err(|e| PyErr::new::<SerializationError, _>(e.to_string()))
    }

    /// Check if database is in-memory
    fn is_memory(&self) -> bool {
        matches!(self.db, DatabaseWrapper::Memory(_))
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
        let stats = self.db.compact().map_err(ironbase_error_to_pyerr)?;

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

    fn __str__(&self) -> String {
        if self.is_memory() {
            "IronBase(:memory:)".to_string()
        } else {
            format!("IronBase('{}')", self.db.path())
        }
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
            .map_err(ironbase_error_to_pyerr)
    }

    /// Rollback a transaction
    fn rollback_transaction(&self, tx_id: u64) -> PyResult<()> {
        self.db
            .rollback_transaction(tx_id)
            .map_err(ironbase_error_to_pyerr)
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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }

    /// Insert many documents within a transaction.
    ///
    /// All documents are inserted atomically - if any insert fails,
    /// the entire transaction can be rolled back.
    ///
    /// Example:
    ///     >>> tx_id = db.begin_transaction()
    ///     >>> try:
    ///     ...     result = db.insert_many_tx("users", [
    ///     ...         {"name": "Alice", "age": 30},
    ///     ...         {"name": "Bob", "age": 25}
    ///     ...     ], tx_id)
    ///     ...     db.commit_transaction(tx_id)
    ///     ... except Exception:
    ///     ...     db.rollback_transaction(tx_id)
    ///     ...     raise
    fn insert_many_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        documents: Bound<'_, PyList>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut inserted_ids = Vec::with_capacity(documents.len());

        for doc in documents.iter() {
            let doc_dict = doc.downcast::<PyDict>()?;
            let mut doc_map: HashMap<String, Value> = HashMap::new();

            for (key, value) in doc_dict.iter() {
                let key_str: String = key.extract()?;
                let json_value = python_to_json(py, &value)?;
                doc_map.insert(key_str, json_value);
            }

            let inserted_id = self
                .db
                .insert_one_tx(&collection_name, doc_map, tx_id)
                .map_err(ironbase_error_to_pyerr)?;

            inserted_ids.push(inserted_id);
        }

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("inserted_count", inserted_ids.len())?;

        let ids_list = PyList::empty(py);
        for doc_id in inserted_ids {
            let id_value = doc_id_to_py(py, &doc_id)?;
            ids_list.append(id_value)?;
        }
        result.set_item("inserted_ids", ids_list)?;

        Ok(result)
    }

    /// Update many documents within a transaction.
    ///
    /// Finds all documents matching the query and applies the update.
    /// All updates happen atomically within the transaction.
    ///
    /// Note: This iterates through matching documents and updates each one.
    /// For very large updates, consider batching or using non-transactional update_many.
    ///
    /// Example:
    ///     >>> tx_id = db.begin_transaction()
    ///     >>> try:
    ///     ...     result = db.update_many_tx(
    ///     ...         "users",
    ///     ...         {"status": "pending"},
    ///     ...         {"$set": {"status": "active"}},
    ///     ...         tx_id
    ///     ...     )
    ///     ...     db.commit_transaction(tx_id)
    ///     ... except Exception:
    ///     ...     db.rollback_transaction(tx_id)
    fn update_many_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        query: Bound<'_, PyDict>,
        update: Bound<'_, PyDict>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;
        let update_json = python_dict_to_json_value(py, &update)?;

        // Get collection to find matching documents
        let collection = self
            .db
            .collection(&collection_name)
            .map_err(ironbase_error_to_pyerr)?;

        // Find all matching document IDs first
        let options = ironbase_core::FindOptions::new().with_limit(100_000);
        let docs = collection
            .find_with_options(&query_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        let mut matched_count: u64 = 0;
        let mut modified_count: u64 = 0;

        // Update each document within the transaction
        for doc in docs {
            if let Some(id) = doc.get("_id") {
                let doc_query = serde_json::json!({"_id": id});
                let (m, mod_c) = self
                    .db
                    .update_one_tx(&collection_name, &doc_query, update_json.clone(), tx_id)
                    .map_err(ironbase_error_to_pyerr)?;
                matched_count += m;
                modified_count += mod_c;
            }
        }

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("matched_count", matched_count)?;
        result.set_item("modified_count", modified_count)?;
        Ok(result)
    }

    /// Delete many documents within a transaction.
    ///
    /// Finds all documents matching the query and deletes them atomically.
    ///
    /// Example:
    ///     >>> tx_id = db.begin_transaction()
    ///     >>> try:
    ///     ...     result = db.delete_many_tx("users", {"status": "inactive"}, tx_id)
    ///     ...     print(f"Deleted {result['deleted_count']} users")
    ///     ...     db.commit_transaction(tx_id)
    ///     ... except Exception:
    ///     ...     db.rollback_transaction(tx_id)
    fn delete_many_tx<'py>(
        &self,
        py: Python<'py>,
        collection_name: String,
        query: Bound<'_, PyDict>,
        tx_id: u64,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;

        // Get collection to find matching documents
        let collection = self
            .db
            .collection(&collection_name)
            .map_err(ironbase_error_to_pyerr)?;

        // Find all matching document IDs first (only need _id)
        let mut options = ironbase_core::FindOptions::new().with_limit(100_000);
        let mut projection = HashMap::new();
        projection.insert("_id".to_string(), 1);
        options.projection = Some(projection);

        let docs = collection
            .find_with_options(&query_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        let mut deleted_count: u64 = 0;

        // Delete each document within the transaction
        for doc in docs {
            if let Some(id) = doc.get("_id") {
                let doc_query = serde_json::json!({"_id": id});
                let count = self
                    .db
                    .delete_one_tx(&collection_name, &doc_query, tx_id)
                    .map_err(ironbase_error_to_pyerr)?;
                deleted_count += count;
            }
        }

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }
}

/// Collection wrapper
#[pyclass]
pub struct Collection {
    core: CollectionWrapper,
    db: DatabaseWrapper,
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
            .map_err(ironbase_error_to_pyerr)
    }

    /// Get current JSON schema
    fn get_schema<'py>(&self, py: Python<'py>) -> PyResult<PyObject> {
        let schema = self.core.get_schema().map_err(ironbase_error_to_pyerr)?;
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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

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

    /// Find documents with options.
    ///
    /// **IMPORTANT:** Default limit is 10,000 documents if not specified.
    /// To fetch more, explicitly set limit (e.g., limit=100000).
    /// For unlimited results, use find_cursor() for streaming.
    ///
    /// Args:
    ///     query: Filter criteria (default: {} = all documents)
    ///     projection: Fields to include/exclude (e.g., {"name": 1, "_id": 0})
    ///     sort: Sort order as list of tuples (e.g., [("age", -1)] for descending)
    ///     limit: Maximum documents to return (default: 10,000)
    ///     skip: Number of documents to skip (for pagination)
    ///
    /// Returns:
    ///     List of document dicts
    ///
    /// Example:
    ///     >>> # Get 100 active users, sorted by age, only name and age fields
    ///     >>> users = coll.find(
    ///     ...     {"status": "active"},
    ///     ...     projection={"name": 1, "age": 1},
    ///     ...     sort=[("age", -1)],
    ///     ...     limit=100
    ///     ... )
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
                if action != 0 && action != 1 {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {}",
                        field, action
                    )));
                }
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

        options.limit = limit.or(Some(DEFAULT_FIND_LIMIT));
        options.skip = skip;

        let results = self
            .core
            .find_with_options(&query_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for doc in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Find documents with total count for pagination (default limit: 10,000 if not provided).
    ///
    /// Returns a dict with 'documents' (list) and 'total' (int).
    ///
    /// Example:
    ///     >>> result = coll.find_with_total({}, limit=10, skip=20)
    ///     >>> print(f"Page: {len(result['documents'])} of {result['total']}")
    #[pyo3(signature = (query=None, projection=None, sort=None, limit=None, skip=None))]
    fn find_with_total<'py>(
        &self,
        py: Python<'py>,
        query: Option<Bound<'_, PyDict>>,
        projection: Option<Bound<'_, PyDict>>,
        sort: Option<Bound<'_, PyList>>,
        limit: Option<usize>,
        skip: Option<usize>,
    ) -> PyResult<Bound<'py, PyDict>> {
        use ironbase_core::find_options::FindOptions;

        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        let mut options = FindOptions::new().with_include_total(true);

        if let Some(proj) = projection {
            let mut projection_map = HashMap::new();
            for (key, value) in proj.iter() {
                let field: String = key.extract()?;
                let action: i32 = value.extract()?;
                if action != 0 && action != 1 {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {}",
                        field, action
                    )));
                }
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

        options.limit = limit.or(Some(DEFAULT_FIND_LIMIT));
        options.skip = skip;

        let result = self
            .core
            .find_with_result(&query_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        let py_dict = PyDict::new(py);

        // Convert documents to Python list
        let py_list = PyList::empty(py);
        for doc in result.documents {
            let doc_dict = json_to_python_dict(py, &doc)?;
            py_list.append(doc_dict)?;
        }
        py_dict.set_item("documents", py_list)?;

        // Set total count
        py_dict.set_item("total", result.total.unwrap_or(0))?;

        Ok(py_dict)
    }

    /// Find one document with optional projection and sort.
    ///
    /// Args:
    ///     query: Filter criteria (default: {})
    ///     projection: Fields to include/exclude (e.g., {"name": 1, "_id": 0})
    ///     sort: Sort order as list of tuples (e.g., [("age", -1)])
    ///
    /// Returns:
    ///     Document dict or None if not found
    ///
    /// Example:
    ///     >>> # Get newest user named Alice, only return name and age
    ///     >>> coll.find_one(
    ///     ...     {"name": "Alice"},
    ///     ...     projection={"name": 1, "age": 1, "_id": 0},
    ///     ...     sort=[("created_at", -1)]
    ///     ... )
    #[pyo3(signature = (query=None, projection=None, sort=None))]
    fn find_one<'py>(
        &self,
        py: Python<'py>,
        query: Option<Bound<'_, PyDict>>,
        projection: Option<Bound<'_, PyDict>>,
        sort: Option<Bound<'_, PyList>>,
    ) -> PyResult<PyObject> {
        use ironbase_core::find_options::FindOptions;

        let query_json = match query {
            Some(q) => python_dict_to_json_value(py, &q)?,
            None => serde_json::json!({}),
        };

        let mut options = FindOptions::new().with_limit(1);

        if let Some(proj) = projection {
            let mut projection_map = HashMap::new();
            for (key, value) in proj.iter() {
                let field: String = key.extract()?;
                let action: i32 = value.extract()?;
                if action != 0 && action != 1 {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid projection value for '{}': expected 0 or 1, got {}",
                        field, action
                    )));
                }
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

        let results = self
            .core
            .find_with_options(&query_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        match results.into_iter().next() {
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
            .map_err(ironbase_error_to_pyerr)
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
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for value in distinct_values {
            let py_value = json_value_to_python(py, &value)?;
            py_list.append(py_value)?;
        }
        Ok(py_list)
    }

    /// Update one document with optional upsert support.
    ///
    /// If upsert=True and no document matches the filter, a new document
    /// is created from the filter criteria and update operations.
    ///
    /// Example:
    ///     >>> result = coll.update_one(
    ///     ...     {"email": "new@example.com"},
    ///     ...     {"$set": {"name": "New User"}},
    ///     ...     upsert=True
    ///     ... )
    ///     >>> if result.get("upserted_id"):
    ///     ...     print(f"Inserted new document: {result['upserted_id']}")
    #[pyo3(signature = (query, update, upsert=false))]
    fn update_one<'py>(
        &self,
        py: Python<'py>,
        query: Bound<'_, PyDict>,
        update: Bound<'_, PyDict>,
        upsert: bool,
    ) -> PyResult<Bound<'py, PyDict>> {
        let query_json = python_dict_to_json_value(py, &query)?;
        let update_json = python_dict_to_json_value(py, &update)?;

        let options = ironbase_core::UpdateOptions::new().with_upsert(upsert);
        let update_result = self
            .db
            .update_one_with_options(&self.name, &query_json, &update_json, options)
            .map_err(ironbase_error_to_pyerr)?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("matched_count", update_result.matched_count)?;
        result.set_item("modified_count", update_result.modified_count)?;

        // Include upserted_id if present
        if let Some(ref doc_id) = update_result.upserted_id {
            let id_value = doc_id_to_py(py, doc_id)?;
            result.set_item("upserted_id", id_value)?;
        }

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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

        let result = PyDict::new(py);
        result.set_item("acknowledged", true)?;
        result.set_item("deleted_count", deleted_count)?;
        Ok(result)
    }

    /// Create an index
    ///
    /// Args:
    ///     field: Field to index
    ///     unique: Whether values must be unique (default: False)
    ///     sparse: If True, documents missing the field are not indexed (default: False)
    #[pyo3(signature = (field, unique=false, sparse=false))]
    fn create_index(&self, field: String, unique: bool, sparse: bool) -> PyResult<String> {
        self.core
            .create_index(field, unique, sparse)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Create a compound index
    ///
    /// Args:
    ///     fields: List of fields to index (in order)
    ///     unique: Whether compound key must be unique (default: False)
    ///     sparse: If True, documents missing any field are not indexed (default: False)
    #[pyo3(signature = (fields, unique=false, sparse=false))]
    fn create_compound_index(
        &self,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> PyResult<String> {
        if fields.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Compound index must have at least one field",
            ));
        }

        self.core
            .create_compound_index(fields, unique, sparse)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Drop an index
    fn drop_index(&self, index_name: String) -> PyResult<()> {
        self.core
            .drop_index(&index_name)
            .map_err(ironbase_error_to_pyerr)
    }

    /// List all indexes
    fn list_indexes(&self) -> PyResult<Vec<String>> {
        self.core.list_indexes().map_err(ironbase_error_to_pyerr)
    }

    // ========== FUZZY SEARCH ==========

    /// Create a fuzzy text index for approximate string matching.
    ///
    /// Args:
    ///     field: Field name to index
    ///     algorithm: "jaro_winkler" (default), "levenshtein", or "damerau_levenshtein"
    ///     threshold: Similarity threshold 0.0-1.0 (default: 0.8)
    ///
    /// Returns:
    ///     Index name (e.g., "collection_field_fuzzy")
    ///
    /// Example:
    ///     >>> coll.create_fuzzy_index("name", algorithm="levenshtein", threshold=0.7)
    #[pyo3(signature = (field, algorithm="jaro_winkler", threshold=0.8))]
    fn create_fuzzy_index(
        &self,
        field: String,
        algorithm: &str,
        threshold: f64,
    ) -> PyResult<String> {
        let algo = match algorithm.to_lowercase().as_str() {
            "jaro_winkler" | "jarowinkler" => FuzzyAlgorithm::JaroWinkler,
            "levenshtein" => FuzzyAlgorithm::Levenshtein,
            "damerau_levenshtein" | "dameraulevenshtein" => FuzzyAlgorithm::DamerauLevenshtein,
            _ => {
                return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Invalid algorithm '{}'. Use 'jaro_winkler', 'levenshtein', or 'damerau_levenshtein'",
                    algorithm
                )));
            }
        };

        self.core
            .create_fuzzy_index(field, algo, threshold)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Search for documents using fuzzy text matching.
    ///
    /// Args:
    ///     field: Field to search (must have a fuzzy index)
    ///     query: Search term
    ///     threshold: Override similarity threshold (0.0-1.0)
    ///     algorithm: Override algorithm ("jaro_winkler", "levenshtein", "damerau_levenshtein")
    ///
    /// Returns:
    ///     List of tuples: (document, similarity_score)
    ///
    /// Example:
    ///     >>> results = coll.fuzzy_search("name", "john", threshold=0.7)
    ///     >>> for doc, score in results:
    ///     ...     print(f"Match: {doc['name']}, Score: {score:.2f}")
    #[pyo3(signature = (field, query, threshold=None, algorithm=None))]
    fn fuzzy_search<'py>(
        &self,
        py: Python<'py>,
        field: String,
        query: String,
        threshold: Option<f64>,
        algorithm: Option<&str>,
    ) -> PyResult<Bound<'py, PyList>> {
        let algo = match algorithm {
            Some(s) => match s.to_lowercase().as_str() {
                "jaro_winkler" | "jarowinkler" => Some(FuzzyAlgorithm::JaroWinkler),
                "levenshtein" => Some(FuzzyAlgorithm::Levenshtein),
                "damerau_levenshtein" | "dameraulevenshtein" => {
                    Some(FuzzyAlgorithm::DamerauLevenshtein)
                }
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                        "Invalid algorithm '{}'. Use 'jaro_winkler', 'levenshtein', or 'damerau_levenshtein'",
                        s
                    )));
                }
            },
            None => None,
        };

        let results = self
            .core
            .fuzzy_search(&field, &query, threshold, algo)
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for (doc, score) in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            let tuple = PyTuple::new(
                py,
                [py_dict.into_any(), score.into_pyobject(py)?.into_any()],
            )?;
            py_list.append(tuple)?;
        }

        Ok(py_list)
    }

    // ========== FULL-TEXT SEARCH ==========

    /// Create a full-text search index with TF-IDF scoring.
    ///
    /// Args:
    ///     field: Field name to index
    ///     language: "english" (default), "hungarian", "german", or "none"
    ///     min_word_length: Minimum word length to index (default: 2)
    ///     accent_folding: Normalize accents (áéí → aei) (default: true)
    ///
    /// Returns:
    ///     Index name (e.g., "collection_field_fulltext")
    ///
    /// Example:
    ///     >>> coll.create_fulltext_index("content", language="hungarian")
    #[pyo3(signature = (field, language="english", min_word_length=None, accent_folding=None))]
    fn create_fulltext_index(
        &self,
        field: String,
        language: &str,
        min_word_length: Option<usize>,
        accent_folding: Option<bool>,
    ) -> PyResult<String> {
        self.core
            .create_fulltext_index(field, language, min_word_length, accent_folding)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Search documents using full-text search with TF-IDF scoring.
    ///
    /// Args:
    ///     field: Field to search (must have a fulltext index)
    ///     query: Search query (words separated by spaces)
    ///     limit: Maximum results (default: 10)
    ///     skip: Number of results to skip (default: 0)
    ///     min_score: Minimum TF-IDF score filter
    ///     projection: Fields to include/exclude (e.g., {"title": 1, "_id": 1})
    ///
    /// Returns:
    ///     List of tuples: (document, score, matched_tokens)
    ///
    /// Example:
    ///     >>> results = coll.fulltext_search("content", "király", limit=10)
    ///     >>> for doc, score, tokens in results:
    ///     ...     print(f"Score: {score:.2f}, Tokens: {tokens}")
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (field, query, limit=None, skip=None, min_score=None, projection=None))]
    fn fulltext_search<'py>(
        &self,
        py: Python<'py>,
        field: String,
        query: String,
        limit: Option<usize>,
        skip: Option<usize>,
        min_score: Option<f64>,
        projection: Option<Bound<'_, PyDict>>,
    ) -> PyResult<Bound<'py, PyList>> {
        let proj_map = match projection {
            Some(ref dict) => {
                let mut map = HashMap::new();
                for (key, value) in dict.iter() {
                    let k: String = key.extract()?;
                    let v: i32 = value.extract()?;
                    if v != 0 && v != 1 {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "Invalid projection value for '{}': expected 0 or 1, got {}",
                            k, v
                        )));
                    }
                    map.insert(k, v);
                }
                Some(map)
            }
            None => None,
        };

        let results = self
            .core
            .fulltext_search(&field, &query, limit, skip, min_score, proj_map)
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for (doc, score, tokens) in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            let py_tokens = PyList::new(py, tokens.iter().map(|s| s.as_str()))?;
            let tuple = PyTuple::new(
                py,
                [
                    py_dict.into_any(),
                    score.into_pyobject(py)?.into_any(),
                    py_tokens.into_any(),
                ],
            )?;
            py_list.append(tuple)?;
        }

        Ok(py_list)
    }

    /// List all fulltext indexes for this collection.
    ///
    /// Returns:
    ///     List of dicts with index metadata (name, field, language, etc.)
    fn list_fulltext_indexes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let indexes = self
            .core
            .list_fulltext_indexes()
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for idx in indexes {
            let dict = PyDict::new(py);
            dict.set_item("name", &idx.name)?;
            dict.set_item("field", &idx.field)?;
            dict.set_item("language", format!("{:?}", idx.language))?;
            dict.set_item("min_word_length", idx.min_word_length)?;
            dict.set_item("accent_folding", idx.accent_folding)?;
            dict.set_item("num_documents", idx.num_documents)?;
            dict.set_item("num_tokens", idx.num_tokens)?;
            py_list.append(dict)?;
        }

        Ok(py_list)
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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

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
            .map_err(ironbase_error_to_pyerr)?;

        let py_list = PyList::empty(py);
        for doc in results {
            let py_dict = json_to_python_dict(py, &doc)?;
            py_list.append(py_dict)?;
        }

        Ok(py_list)
    }

    /// Execute aggregation pipeline with automatic memory-safe limits.
    ///
    /// This method automatically scales memory limits based on available system RAM.
    /// Use this instead of aggregate() for large datasets to prevent OOM errors.
    ///
    /// Memory scaling (approximate):
    ///   - < 512 MB RAM: 64 MB limit, 10K docs, 5K groups
    ///   - 512 MB - 2 GB: 128 MB limit, 50K docs, 25K groups
    ///   - 2 GB - 8 GB: 256 MB limit, 100K docs, 50K groups
    ///   - 8 GB - 32 GB: 512 MB limit, 250K docs, 100K groups
    ///   - > 32 GB: 1024 MB limit, 500K docs, 250K groups
    ///
    /// Example:
    ///     >>> # Safe aggregation that won't OOM
    ///     >>> results = coll.aggregate_auto([
    ///     ...     {"$match": {"status": "active"}},
    ///     ...     {"$group": {"_id": "$category", "count": {"$sum": 1}}},
    ///     ...     {"$sort": {"count": -1}},
    ///     ...     {"$limit": 100}
    ///     ... ])
    ///
    /// Raises:
    ///     OutOfMemoryError: If aggregation exceeds memory limits
    ///     AggregationError: If pipeline is invalid
    fn aggregate_auto<'py>(
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
            .aggregate_auto(&pipeline_json)
            .map_err(ironbase_error_to_pyerr)?;

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
            db: self.db.clone(),
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
        format!("Collection('{}')", self.core.name())
    }
}

/// Lazy cursor for iterating through query results
///
/// Documents are loaded in batches as you iterate, not all at once.
/// This allows processing millions of documents without memory exhaustion.
#[pyclass]
pub struct Cursor {
    db: DatabaseWrapper,
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
            .map_err(ironbase_error_to_pyerr)?;

        let options = ironbase_core::FindOptions::new()
            .with_skip(self.position)
            .with_limit(self.batch_size);

        let results = collection
            .find_with_options(&self.query, options)
            .map_err(ironbase_error_to_pyerr)?;

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

    /// Get total count of documents matching the query.
    ///
    /// Note: This runs a separate count query against the database.
    /// The count is for the original query, not remaining documents.
    ///
    /// Example:
    ///     >>> cursor = coll.find_cursor({"status": "active"})
    ///     >>> total = cursor.count()  # Total matching documents
    ///     >>> for doc in cursor:
    ///     ...     print(f"Processing {cursor.position()} of {total}")
    fn count(&self) -> PyResult<u64> {
        let collection = self
            .db
            .collection(&self.collection_name)
            .map_err(ironbase_error_to_pyerr)?;

        collection
            .count_documents(&self.query)
            .map_err(ironbase_error_to_pyerr)
    }

    /// Check if cursor has more documents (for bool() conversion).
    ///
    /// Returns True if there are more documents to iterate.
    ///
    /// Example:
    ///     >>> cursor = coll.find_cursor({"status": "active"})
    ///     >>> if cursor:
    ///     ...     print("Has documents")
    fn __bool__(&self) -> bool {
        !self.is_finished()
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
        // JSON does not support NaN or Infinity - raise error instead of silent conversion
        if f.is_nan() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Cannot convert NaN to JSON. Use None/null instead.",
            ));
        }
        if f.is_infinite() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Cannot convert Infinity to JSON. Use None/null or a large number instead.",
            ));
        }
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
    // Classes
    m.add_class::<IronBase>()?;
    m.add_class::<Collection>()?;
    m.add_class::<Cursor>()?;

    // Exceptions - allows: except ironbase.CollectionNotFoundError
    m.add("IronBaseException", m.py().get_type::<IronBaseException>())?;
    m.add(
        "CollectionNotFoundError",
        m.py().get_type::<CollectionNotFoundError>(),
    )?;
    m.add(
        "CollectionExistsError",
        m.py().get_type::<CollectionExistsError>(),
    )?;
    m.add(
        "DocumentNotFoundError",
        m.py().get_type::<DocumentNotFoundError>(),
    )?;
    m.add("InvalidQueryError", m.py().get_type::<InvalidQueryError>())?;
    m.add("CorruptionError", m.py().get_type::<CorruptionError>())?;
    m.add("IndexError", m.py().get_type::<IndexError>())?;
    m.add("AggregationError", m.py().get_type::<AggregationError>())?;
    m.add(
        "SchemaValidationError",
        m.py().get_type::<SchemaValidationError>(),
    )?;
    m.add("TransactionError", m.py().get_type::<TransactionError>())?;
    m.add(
        "DatabaseLockedError",
        m.py().get_type::<DatabaseLockedError>(),
    )?;
    m.add(
        "DatabaseClosedError",
        m.py().get_type::<DatabaseClosedError>(),
    )?;
    m.add(
        "OperationNotAllowedError",
        m.py().get_type::<OperationNotAllowedError>(),
    )?;
    m.add("OutOfMemoryError", m.py().get_type::<OutOfMemoryError>())?;
    m.add("CancelledError", m.py().get_type::<CancelledError>())?;
    m.add("TimeoutError", m.py().get_type::<TimeoutError>())?;
    m.add(
        "SerializationError",
        m.py().get_type::<SerializationError>(),
    )?;

    Ok(())
}
