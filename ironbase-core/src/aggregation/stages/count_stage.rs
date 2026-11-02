// src/aggregation/stages/count_stage.rs
// $count stage implementation

use crate::aggregation::types::CountStage;
use crate::error::{IronBaseError, Result};
use serde_json::Value;

/// The output of a count over `input_docs` documents: one `{field: value}`
/// document (with `_id: null` first when `include_id`), or none for an empty
/// input. MongoDB semantics: `$count` is sugar for
/// `{$group: {_id: null, n: {$sum: 1}}}, {$project: {_id: 0}}`, and a
/// `_id: null` `$group` emits nothing for zero input rows. Every count path
/// (this stage, the streaming pipeline, the count-only fast path) builds its
/// result here (#107).
pub(crate) fn count_output(
    field: &str,
    input_docs: u64,
    value: i64,
    include_id: bool,
) -> Vec<Value> {
    if input_docs == 0 {
        return Vec::new();
    }
    let mut doc = serde_json::Map::new();
    if include_id {
        doc.insert("_id".to_string(), Value::Null);
    }
    doc.insert(field.to_string(), Value::from(value));
    vec![Value::Object(doc)]
}

impl CountStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Some(field) = spec.as_str() {
            if field.is_empty() {
                return Err(IronBaseError::AggregationError(
                    "$count field name must not be empty".to_string(),
                ));
            }
            // As in MongoDB: the output field is a plain name (#107)
            if field.starts_with('$') || field.contains('.') {
                return Err(IronBaseError::AggregationError(format!(
                    "$count field name must not start with '$' or contain '.': {}",
                    field
                )));
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
        Ok(self.output(docs.len() as u64))
    }

    /// The `$count` result for `count` input documents
    pub(crate) fn output(&self, count: u64) -> Vec<Value> {
        count_output(&self.field, count, count as i64, false)
    }
}
