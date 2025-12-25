// src/aggregation/stages/sort_stage.rs
// $sort stage implementation

use crate::aggregation::types::{SortDirection, SortStage};
use crate::error::{IronBaseError, Result};
use crate::value_utils::{compare_values as compare_values_core, get_nested_value};
use serde_json::Value;

impl SortStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let mut fields = Vec::new();

            for (field, value) in obj {
                let direction = if let Some(n) = value.as_i64() {
                    match n {
                        1 => SortDirection::Ascending,
                        -1 => SortDirection::Descending,
                        _ => {
                            return Err(IronBaseError::AggregationError(
                                "Sort direction must be 1 or -1".to_string(),
                            ))
                        }
                    }
                } else {
                    return Err(IronBaseError::AggregationError(
                        "Sort direction must be 1 or -1".to_string(),
                    ));
                };

                fields.push((field.clone(), direction));
            }

            Ok(SortStage { fields })
        } else {
            Err(IronBaseError::AggregationError(
                "$sort must be an object".to_string(),
            ))
        }
    }

    pub(crate) fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        docs.sort_by(|a, b| {
            for (field, direction) in &self.fields {
                let val_a = get_nested_value(a, field);
                let val_b = get_nested_value(b, field);

                let cmp = compare_values(val_a, val_b);
                let cmp = match direction {
                    SortDirection::Ascending => cmp,
                    SortDirection::Descending => cmp.reverse(),
                };

                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        });

        Ok(docs)
    }
}

fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    match (a, b) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(a_val), Some(b_val)) => {
            compare_values_core(a_val, b_val).unwrap_or(std::cmp::Ordering::Equal)
        }
    }
}
