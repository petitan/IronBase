// src/aggregation/stages/limit_stage.rs
// $limit stage implementation

use crate::aggregation::types::LimitStage;
use crate::error::{MongoLiteError, Result};
use serde_json::Value;

impl LimitStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Some(n) = spec.as_u64() {
            Ok(LimitStage { limit: n as usize })
        } else {
            Err(MongoLiteError::AggregationError(
                "$limit must be a positive number".to_string(),
            ))
        }
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().take(self.limit).collect())
    }
}
