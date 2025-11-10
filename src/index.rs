// src/index.rs
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::document::DocumentId;
use crate::error::{Result, MongoLiteError};

/// Index típusok
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexType {
    /// Normál index
    Regular,
    /// Egyedi értékek indexe
    Unique,
    /// Szöveges keresés
    Text,
    /// Geospatial index
    Geo2d,
}

/// Index definíció
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub field: String,
    pub index_type: IndexType,
    pub unique: bool,
}

/// Index tárolás - egyszerű HashMap alapú
pub struct Index {
    definition: IndexDefinition,
    // Mező érték -> Dokumentum ID lista
    entries: HashMap<String, Vec<DocumentId>>,
}

impl Index {
    /// Új index létrehozása
    pub fn new(definition: IndexDefinition) -> Self {
        Index {
            definition,
            entries: HashMap::new(),
        }
    }
    
    /// Érték hozzáadása az indexhez
    pub fn insert(&mut self, key: String, doc_id: DocumentId) -> Result<()> {
        if self.definition.unique && self.entries.contains_key(&key) {
            return Err(MongoLiteError::IndexError(
                format!("Duplicate key: {} (unique index)", key)
            ));
        }
        
        self.entries.entry(key)
            .or_insert_with(Vec::new)
            .push(doc_id);
        
        Ok(())
    }
    
    /// Keresés az indexben
    pub fn find(&self, key: &str) -> Option<&Vec<DocumentId>> {
        self.entries.get(key)
    }
    
    /// Érték törlése az indexből
    pub fn remove(&mut self, key: &str, doc_id: &DocumentId) {
        if let Some(ids) = self.entries.get_mut(key) {
            ids.retain(|id| id != doc_id);
            if ids.is_empty() {
                self.entries.remove(key);
            }
        }
    }
    
    /// Index mérete (bejegyzések száma)
    pub fn size(&self) -> usize {
        self.entries.len()
    }
}

/// Index Manager - collection-höz tartozó indexek kezelése
pub struct IndexManager {
    indexes: HashMap<String, Index>,
}

impl IndexManager {
    pub fn new() -> Self {
        IndexManager {
            indexes: HashMap::new(),
        }
    }
    
    /// Index létrehozása
    pub fn create_index(&mut self, definition: IndexDefinition) -> Result<()> {
        let name = definition.name.clone();
        
        if self.indexes.contains_key(&name) {
            return Err(MongoLiteError::IndexError(
                format!("Index already exists: {}", name)
            ));
        }
        
        self.indexes.insert(name, Index::new(definition));
        Ok(())
    }
    
    /// Index törlése
    pub fn drop_index(&mut self, name: &str) -> Result<()> {
        self.indexes.remove(name)
            .ok_or_else(|| MongoLiteError::IndexError(
                format!("Index not found: {}", name)
            ))?;
        Ok(())
    }
    
    /// Index lekérése
    pub fn get_index(&self, name: &str) -> Option<&Index> {
        self.indexes.get(name)
    }
    
    /// Index lekérése (módosítható)
    pub fn get_index_mut(&mut self, name: &str) -> Option<&mut Index> {
        self.indexes.get_mut(name)
    }
    
    /// Indexek listája
    pub fn list_indexes(&self) -> Vec<String> {
        self.indexes.keys().cloned().collect()
    }
}

impl Default for IndexManager {
    fn default() -> Self {
        Self::new()
    }
}
