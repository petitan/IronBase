// src/aggregation/stages/count_stage.rs
// $count stage implementation

use crate::aggregation::types::CountStage;
use crate::error::{IronBaseError, Result};
use serde_json::Value;

impl CountStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Some(field) = spec.as_str() {
            if field.is_empty() {
                return Err(IronBaseError::AggregationError(
                    "$count field name must not be empty".to_string(),
                ));
            }
            Ok(CountStage {
                field: field.to_string(),
            })
        } else {
            Err(IronBaseError::AggregationError(
                "$count must be a string field name".to_string(),
            ))
        }
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let count = docs.len() as i64;
        Ok(vec![serde_json::json!({
            self.field.clone(): count
        })])
    }
}
