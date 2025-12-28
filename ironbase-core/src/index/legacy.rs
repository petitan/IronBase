// Legacy HashMap-based Index (for compatibility)

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Index types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexType {
    Regular,
    Unique,
    Text,
    Geo2d,
}

/// Index definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDefinition {
    pub name: String,
    pub field: String,
    pub index_type: IndexType,
    pub unique: bool,
}

/// Simple HashMap-based index (legacy)
pub struct Index {
    pub(crate) definition: IndexDefinition,
    entries: HashMap<String, Vec<DocumentId>>,
}

impl Index {
    pub fn new(definition: IndexDefinition) -> Self {
        Index {
            definition,
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: String, doc_id: DocumentId) -> Result<()> {
        if self.definition.unique && self.entries.contains_key(&key) {
            return Err(IronBaseError::IndexError(format!(
                "Duplicate key: {} (unique index)",
                key
            )));
        }

        self.entries.entry(key).or_default().push(doc_id);

        Ok(())
    }

    pub fn find(&self, key: &str) -> Option<&Vec<DocumentId>> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &str, doc_id: &DocumentId) {
        if let Some(ids) = self.entries.get_mut(key) {
            ids.retain(|id| id != doc_id);
            if ids.is_empty() {
                self.entries.remove(key);
            }
        }
    }

    pub fn size(&self) -> usize {
        self.entries.len()
    }
}
