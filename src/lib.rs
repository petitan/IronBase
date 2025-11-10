// src/lib.rs
use pyo3::prelude::*;
use std::sync::Arc;
use parking_lot::RwLock;

mod storage;
mod collection;
mod query;
mod index;
mod document;
mod error;

pub use error::{MongoLiteError, Result};
pub use document::{Document, DocumentId};
pub use collection::Collection;
pub use storage::StorageEngine;

/// MongoLite Database - fő osztály
#[pyclass]
pub struct MongoLite {
    storage: Arc<RwLock<StorageEngine>>,
    db_path: String,
}

#[pymethods]
impl MongoLite {
    /// Új adatbázis megnyitása vagy létrehozása
    /// 
    /// Args:
    ///     path: Az adatbázis fájl elérési útja
    /// 
    /// Example:
    ///     db = MongoLite("myapp.mlite")
    #[new]
    fn new(path: String) -> PyResult<Self> {
        let storage = StorageEngine::open(&path)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        
        Ok(MongoLite {
            storage: Arc::new(RwLock::new(storage)),
            db_path: path,
        })
    }
    
    /// Collection lekérése (ha nem létezik, létrehozza)
    /// 
    /// Args:
    ///     name: A collection neve
    /// 
    /// Returns:
    ///     Collection objektum
    fn collection(&self, name: String) -> PyResult<Collection> {
        Collection::new(name, Arc::clone(&self.storage))
    }
    
    /// Collection-ök listája
    fn list_collections(&self) -> PyResult<Vec<String>> {
        let storage = self.storage.read();
        Ok(storage.list_collections())
    }
    
    /// Collection törlése
    fn drop_collection(&self, name: String) -> PyResult<()> {
        let mut storage = self.storage.write();
        storage.drop_collection(&name)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
    }
    
    /// Adatbázis bezárása és flush
    fn close(&self) -> PyResult<()> {
        let mut storage = self.storage.write();
        storage.flush()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))
    }
    
    /// Adatbázis statisztikák
    fn stats(&self) -> PyResult<String> {
        let storage = self.storage.read();
        Ok(serde_json::to_string_pretty(&storage.stats()).unwrap())
    }
    
    fn __repr__(&self) -> String {
        format!("MongoLite('{}')", self.db_path)
    }
}

/// Python modul inicializálás
#[pymodule]
fn mongolite(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<MongoLite>()?;
    m.add_class::<Collection>()?;
    Ok(())
}
