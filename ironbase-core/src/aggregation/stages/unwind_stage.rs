// src/aggregation/stages/unwind_stage.rs
// $unwind stage implementation

use crate::aggregation::types::UnwindStage;
use crate::error::{IronBaseError, Result};
use crate::value_utils::{get_nested_value, set_nested_value};
use serde_json::Value;

impl UnwindStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        // Simple form: "$fieldName"
        if let Some(s) = spec.as_str() {
            if s.starts_with('$') {
                return Ok(UnwindStage {
                    path: s.trim_start_matches('$').to_string(),
                    include_array_index: None,
                    preserve_null_and_empty_arrays: false,
                });
            }
            return Err(IronBaseError::AggregationError(
                "$unwind path must start with $".to_string(),
            ));
        }

        // Extended form: {path: "$fieldName", ...}
        if let Value::Object(obj) = spec {
            let path = obj.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                IronBaseError::AggregationError("$unwind requires 'path' field".to_string())
            })?;

            if !path.starts_with('$') {
                return Err(IronBaseError::AggregationError(
                    "$unwind path must start with $".to_string(),
                ));
            }

            let include_array_index = obj
                .get("includeArrayIndex")
                .and_then(|v| v.as_str())
                .map(String::from);

            let preserve_null_and_empty_arrays = obj
                .get("preserveNullAndEmptyArrays")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            return Ok(UnwindStage {
                path: path.trim_start_matches('$').to_string(),
                include_array_index,
                preserve_null_and_empty_arrays,
            });
        }

        Err(IronBaseError::AggregationError(
            "$unwind must be a string or object".to_string(),
        ))
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let array_value = get_nested_value(&doc, &self.path);

            match array_value {
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    for (index, element) in arr.iter().enumerate() {
                        let mut new_doc = doc.clone();

                        set_nested_value(&mut new_doc, &self.path, element.clone());

                        if let Some(ref index_field) = self.include_array_index {
                            set_nested_value(
                                &mut new_doc,
                                index_field,
                                Value::Number(serde_json::Number::from(index)),
                            );
                        }

                        results.push(new_doc);
                    }
                }
                Some(Value::Array(_)) => {
                    if self.preserve_null_and_empty_arrays {
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                None => {
                    if self.preserve_null_and_empty_arrays {
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                Some(Value::Null) => {
                    if self.preserve_null_and_empty_arrays {
                        results.push(doc);
                    }
                }
                Some(_) => {
                    if let Some(ref index_field) = self.include_array_index {
                        let mut new_doc = doc.clone();
                        set_nested_value(
                            &mut new_doc,
                            index_field,
                            Value::Number(serde_json::Number::from(0)),
                        );
                        results.push(new_doc);
                    } else {
                        results.push(doc);
                    }
                }
            }
        }

        Ok(results)
    }
}
