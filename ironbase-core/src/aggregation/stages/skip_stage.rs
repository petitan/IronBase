// src/aggregation/stages/skip_stage.rs
// $skip stage implementation

use crate::aggregation::types::SkipStage;
use crate::error::{MongoLiteError, Result};
use serde_json::Value;

impl SkipStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Some(n) = spec.as_u64() {
            Ok(SkipStage { skip: n as usize })
        } else {
            Err(MongoLiteError::AggregationError(
                "$skip must be a positive number".to_string(),
            ))
        }
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().skip(self.skip).collect())
    }
}
