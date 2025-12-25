// src/aggregation/stages/accumulator.rs
// Accumulator implementations for $group stage

use crate::aggregation::helpers::{compute_extremum, parse_field_reference};
use crate::aggregation::types::{Accumulator, SumExpression};
use crate::error::{IronBaseError, Result};
use crate::value_utils::{canonical_json_string, get_nested_value};
use serde_json::Value;
use std::collections::HashSet;

impl Accumulator {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            if obj.len() != 1 {
                return Err(IronBaseError::AggregationError(
                    "Accumulator must have exactly one operator".to_string(),
                ));
            }

            let (op, value) = obj.iter().next().unwrap();

            match op.as_str() {
                "$sum" => {
                    if let Some(n) = value.as_i64() {
                        Ok(Accumulator::Sum(SumExpression::Constant(n)))
                    } else if let Some(s) = value.as_str() {
                        if s.starts_with('$') {
                            Ok(Accumulator::Sum(SumExpression::Field(
                                s.trim_start_matches('$').to_string(),
                            )))
                        } else {
                            Err(IronBaseError::AggregationError(
                                "$sum field reference must start with $".to_string(),
                            ))
                        }
                    } else {
                        Err(IronBaseError::AggregationError(
                            "$sum must be a number or field reference".to_string(),
                        ))
                    }
                }
                "$avg" => Ok(Accumulator::Avg(parse_field_reference(value, "$avg")?)),
                "$min" => Ok(Accumulator::Min(parse_field_reference(value, "$min")?)),
                "$max" => Ok(Accumulator::Max(parse_field_reference(value, "$max")?)),
                "$first" => Ok(Accumulator::First(parse_field_reference(value, "$first")?)),
                "$last" => Ok(Accumulator::Last(parse_field_reference(value, "$last")?)),
                "$push" => Ok(Accumulator::Push(parse_field_reference(value, "$push")?)),
                "$addToSet" => Ok(Accumulator::AddToSet(parse_field_reference(
                    value,
                    "$addToSet",
                )?)),
                _ => Err(IronBaseError::AggregationError(format!(
                    "Unknown accumulator: {}",
                    op
                ))),
            }
        } else {
            Err(IronBaseError::AggregationError(
                "Accumulator must be an object".to_string(),
            ))
        }
    }

    pub(crate) fn compute(&self, docs: &[Value]) -> Result<Value> {
        match self {
            Accumulator::Sum(expr) => match expr {
                SumExpression::Constant(n) => {
                    Ok(Value::from((*n).saturating_mul(docs.len() as i64)))
                }
                SumExpression::Field(field) => {
                    let mut sum_int: i64 = 0;
                    let mut sum_float: f64 = 0.0;
                    let mut has_float = false;

                    for doc in docs {
                        if let Some(value) = get_nested_value(doc, field) {
                            if let Some(n) = value.as_i64() {
                                sum_int = sum_int.saturating_add(n);
                            } else if let Some(f) = value.as_f64() {
                                sum_float += f;
                                has_float = true;
                            }
                        }
                    }

                    if has_float {
                        Ok(Value::from(sum_float + sum_int as f64))
                    } else {
                        Ok(Value::from(sum_int))
                    }
                }
            },

            Accumulator::Avg(field) => {
                let mut sum = 0.0;
                let mut count: usize = 0;

                for doc in docs {
                    if let Some(value) = get_nested_value(doc, field) {
                        if let Some(n) = value.as_f64() {
                            sum += n;
                            count = count.saturating_add(1);
                        } else if let Some(n) = value.as_i64() {
                            sum += n as f64;
                            count = count.saturating_add(1);
                        }
                    }
                }

                if count > 0 {
                    Ok(Value::from(sum / count as f64))
                } else {
                    Ok(Value::Null)
                }
            }

            Accumulator::Min(field) => compute_extremum(docs, field, true),

            Accumulator::Max(field) => compute_extremum(docs, field, false),

            Accumulator::First(field) => Ok(docs
                .first()
                .and_then(|doc| get_nested_value(doc, field).cloned())
                .unwrap_or(Value::Null)),

            Accumulator::Last(field) => Ok(docs
                .last()
                .and_then(|doc| get_nested_value(doc, field).cloned())
                .unwrap_or(Value::Null)),

            Accumulator::Push(field) => {
                let values: Vec<Value> = docs
                    .iter()
                    .filter_map(|doc| get_nested_value(doc, field).cloned())
                    .collect();
                Ok(Value::Array(values))
            }

            Accumulator::AddToSet(field) => {
                let mut seen = HashSet::new();
                let mut values = Vec::new();

                for doc in docs {
                    if let Some(value) = get_nested_value(doc, field) {
                        let key = canonical_json_string(value);
                        if seen.insert(key) {
                            values.push(value.clone());
                        }
                    }
                }

                Ok(Value::Array(values))
            }
        }
    }
}
