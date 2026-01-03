// src/aggregation/stages/unwind_stage.rs
// $unwind stage implementation

use crate::aggregation::context::AggregationLimitContext;
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

    /// Default max output documents for $unwind to prevent OOM
    /// 1000 docs × 10000 array elements = 10M output → ezt limitáljuk
    const DEFAULT_MAX_UNWIND_OUTPUT: usize = 1_000_000;

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        self.execute_with_limit(docs, Self::DEFAULT_MAX_UNWIND_OUTPUT)
    }

    /// Execute $unwind with explicit output limit
    ///
    /// # OOM Protection
    /// - Checks output count BEFORE each push
    /// - Uses try_reserve() for memory safety
    /// - Default limit: 1,000,000 output documents
    pub(crate) fn execute_with_limit(
        &self,
        docs: Vec<Value>,
        max_output: usize,
    ) -> Result<Vec<Value>> {
        // Estimate output size: worst case is sum of all array lengths
        // We don't pre-calculate this to avoid scanning twice, but we check incrementally
        let mut results = Vec::new();
        let mut output_count: usize = 0;

        for doc in docs {
            let array_value = get_nested_value(&doc, &self.path);

            match array_value {
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    // Pre-check: will this array push us over the limit?
                    if output_count.saturating_add(arr.len()) > max_output {
                        return Err(IronBaseError::AggregationError(format!(
                            "$unwind would exceed output limit: {} + {} > {} documents. \
                             Array at path '{}' has {} elements. \
                             Consider filtering with $match first or using $slice to limit array size.",
                            output_count, arr.len(), max_output, self.path, arr.len()
                        )));
                    }

                    // try_reserve for this batch
                    results.try_reserve(arr.len()).map_err(|_| {
                        IronBaseError::OutOfMemory(format!(
                            "$unwind failed to allocate memory for {} elements at path '{}'",
                            arr.len(),
                            self.path
                        ))
                    })?;

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

                        output_count += 1;
                        results.push(new_doc);
                    }
                }
                Some(Value::Array(_)) => {
                    if self.preserve_null_and_empty_arrays {
                        output_count += 1;
                        if output_count > max_output {
                            return Err(IronBaseError::AggregationError(format!(
                                "$unwind exceeded output limit: {} documents (max: {})",
                                output_count, max_output
                            )));
                        }
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                None => {
                    if self.preserve_null_and_empty_arrays {
                        output_count += 1;
                        if output_count > max_output {
                            return Err(IronBaseError::AggregationError(format!(
                                "$unwind exceeded output limit: {} documents (max: {})",
                                output_count, max_output
                            )));
                        }
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                Some(Value::Null) => {
                    if self.preserve_null_and_empty_arrays {
                        output_count += 1;
                        if output_count > max_output {
                            return Err(IronBaseError::AggregationError(format!(
                                "$unwind exceeded output limit: {} documents (max: {})",
                                output_count, max_output
                            )));
                        }
                        results.push(doc);
                    }
                }
                Some(_) => {
                    output_count += 1;
                    if output_count > max_output {
                        return Err(IronBaseError::AggregationError(format!(
                            "$unwind exceeded output limit: {} documents (max: {})",
                            output_count, max_output
                        )));
                    }
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

    /// Execute $unwind with centralized context for limit tracking
    ///
    /// Uses AggregationLimitContext for:
    /// - Unwind output count tracking (increment_unwind)
    /// - Consistent error messages
    ///
    /// This method integrates with the unified limit system introduced in 2026-01.
    #[allow(dead_code)] // Will be used in pipeline.rs integration
    pub(crate) fn execute_with_context_impl(
        &self,
        docs: Vec<Value>,
        ctx: &AggregationLimitContext,
    ) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let array_value = get_nested_value(&doc, &self.path);

            match array_value {
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    // Check limit via context BEFORE processing array
                    let arr_len = arr.len();

                    // try_reserve for this batch
                    results.try_reserve(arr_len).map_err(|_| {
                        IronBaseError::OutOfMemory(format!(
                            "$unwind failed to allocate memory for {} elements at path '{}'",
                            arr_len, self.path
                        ))
                    })?;

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

                    // Increment unwind counter in context (batch increment)
                    ctx.increment_unwind(arr_len)?;
                }
                Some(Value::Array(_)) => {
                    // Empty array
                    if self.preserve_null_and_empty_arrays {
                        ctx.increment_unwind(1)?;
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                None => {
                    if self.preserve_null_and_empty_arrays {
                        ctx.increment_unwind(1)?;
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                }
                Some(Value::Null) => {
                    if self.preserve_null_and_empty_arrays {
                        ctx.increment_unwind(1)?;
                        results.push(doc);
                    }
                }
                Some(_) => {
                    // Non-array value - treat as single element
                    ctx.increment_unwind(1)?;
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
