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
        // MongoDB semantics: $count over an EMPTY input set produces NO document.
        // $count is sugar for `{$group:{_id:null,n:{$sum:1}}},{$project:{_id:0}}`,
        // and a `_id: null` $group emits nothing for zero input rows. Match the
        // count-only $group path, which also returns [] for empty input.
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let count = docs.len() as i64;
        Ok(vec![serde_json::json!({
            self.field.clone(): count
        })])
    }
}
