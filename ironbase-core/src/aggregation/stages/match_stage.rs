// src/aggregation/stages/match_stage.rs
// $match stage implementation

use crate::aggregation::types::MatchStage;
use crate::document::Document;
use crate::error::Result;
use crate::query::Query;
use serde_json::Value;

impl MatchStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        let query = Query::from_json(spec)?;
        Ok(MatchStage { query })
    }

    /// Check if a single document matches this stage's query
    ///
    /// Used for streaming execution where we filter documents one at a time.
    pub(crate) fn matches(&self, doc: &Value) -> Result<bool> {
        // Add _id if not present (for aggregation intermediate results)
        let doc_with_id = if doc.get("_id").is_none() {
            let mut doc_obj = doc.clone();
            if let Value::Object(ref mut map) = doc_obj {
                map.insert("_id".to_string(), Value::from(0)); // Temporary _id
            }
            doc_obj
        } else {
            doc.clone()
        };

        // Convert Value directly to Document (no JSON roundtrip)
        let document = Document::from_value(&doc_with_id)?;

        self.query.matches(&document)
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            if self.matches(&doc)? {
                results.push(doc);
            }
        }

        Ok(results)
    }
}
