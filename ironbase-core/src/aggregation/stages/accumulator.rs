// src/aggregation/stages/accumulator.rs
// Accumulator implementations for $group stage
//
// STREAMING OPTIMIZATION (2024-12):
// The accumulator now supports streaming/incremental updates to prevent OOM.
// Instead of storing all documents per group, we maintain only the accumulated state.
// Memory: O(N * doc_size) → O(G * state_size) where G = number of groups

use crate::aggregation::context::AggregationLimitContext;
use crate::aggregation::helpers::{parse_field_reference, parse_value_expression};
use crate::aggregation::types::{Accumulator, AccumulatorState, SumExpression, ValueExpression};
use crate::error::{IronBaseError, Result};
use crate::value_utils::{canonical_json_string, compare_values, get_nested_value, substr_string};
use serde_json::Value;
use std::collections::HashSet;

fn evaluate_value_expr(doc: &Value, expr: &ValueExpression) -> Option<Value> {
    match expr {
        ValueExpression::Field(field) => get_nested_value(doc, field).cloned(),
        ValueExpression::Substr {
            field,
            start,
            length,
        } => {
            let raw = get_nested_value(doc, field)?;
            raw.as_str()
                .map(|text| Value::String(substr_string(text, *start, *length)))
        }
    }
}

fn expr_debug_name(expr: &ValueExpression) -> String {
    match expr {
        ValueExpression::Field(field) => field.clone(),
        ValueExpression::Substr {
            field,
            start,
            length,
        } => format!("$substr(${},{},{})", field, start, length),
    }
}

fn compute_extremum_expr(docs: &[Value], expr: &ValueExpression, is_min: bool) -> Result<Value> {
    let mut result: Option<Value> = None;

    for doc in docs {
        if let Some(value) = evaluate_value_expr(doc, expr) {
            match &result {
                None => result = Some(value),
                Some(current) => {
                    if compare_values(&value, current)
                        == Some(if is_min {
                            std::cmp::Ordering::Less
                        } else {
                            std::cmp::Ordering::Greater
                        })
                    {
                        result = Some(value);
                    }
                }
            }
        }
    }

    Ok(result.unwrap_or(Value::Null))
}

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
                "$min" => Ok(Accumulator::Min(parse_value_expression(value, "$min")?)),
                "$max" => Ok(Accumulator::Max(parse_value_expression(value, "$max")?)),
                "$first" => Ok(Accumulator::First(parse_value_expression(value, "$first")?)),
                "$last" => Ok(Accumulator::Last(parse_value_expression(value, "$last")?)),
                "$push" => Ok(Accumulator::Push(parse_value_expression(value, "$push")?)),
                "$addToSet" => Ok(Accumulator::AddToSet(parse_value_expression(
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

    /// DEPRECATED: Use streaming accumulators (init_state + update + finalize) instead.
    /// This method loads all documents into memory and is kept for backwards compatibility.
    #[allow(dead_code)]
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

            Accumulator::Min(expr) => compute_extremum_expr(docs, expr, true),

            Accumulator::Max(expr) => compute_extremum_expr(docs, expr, false),

            Accumulator::First(expr) => Ok(docs
                .first()
                .and_then(|doc| evaluate_value_expr(doc, expr))
                .unwrap_or(Value::Null)),

            Accumulator::Last(expr) => Ok(docs
                .last()
                .and_then(|doc| evaluate_value_expr(doc, expr))
                .unwrap_or(Value::Null)),

            Accumulator::Push(expr) => {
                let values: Vec<Value> = docs
                    .iter()
                    .filter_map(|doc| evaluate_value_expr(doc, expr))
                    .collect();
                Ok(Value::Array(values))
            }

            Accumulator::AddToSet(expr) => {
                let mut seen = HashSet::new();
                let mut values = Vec::new();

                for doc in docs {
                    if let Some(value) = evaluate_value_expr(doc, expr) {
                        let key = canonical_json_string(&value);
                        if seen.insert(key) {
                            values.push(value.clone());
                        }
                    }
                }

                Ok(Value::Array(values))
            }
        }
    }

    /// Initialize streaming accumulator state from accumulator definition
    pub(crate) fn init_state(&self) -> AccumulatorState {
        match self {
            Accumulator::Sum(expr) => AccumulatorState::Sum {
                int_sum: 0,
                float_sum: 0.0,
                has_float: false,
                is_count: matches!(expr, SumExpression::Constant(_)),
            },
            Accumulator::Avg(_) => AccumulatorState::Avg { sum: 0.0, count: 0 },
            Accumulator::Min(_) => AccumulatorState::Min { value: None },
            Accumulator::Max(_) => AccumulatorState::Max { value: None },
            Accumulator::First(_) => AccumulatorState::First {
                value: None,
                captured: false,
            },
            Accumulator::Last(_) => AccumulatorState::Last {
                value: None,
                doc_count: 0,
            },
            Accumulator::Push(_) => AccumulatorState::Push { values: Vec::new() },
            Accumulator::AddToSet(_) => AccumulatorState::AddToSet {
                seen: HashSet::new(),
                values: Vec::new(),
            },
        }
    }
}

impl AccumulatorState {
    /// Update state with a single document (streaming/incremental)
    /// This is the key optimization - we don't store the document, only update state
    ///
    /// **WARNING:** This method has NO LIMITS on $push/$addToSet!
    /// Use `update_with_limits()` for OOM protection.
    pub(crate) fn update(&mut self, doc: &Value, accumulator: &Accumulator) -> Result<()> {
        // Delegate to unlimited version (backwards compatibility)
        // Propagate errors (e.g. try_reserve OOM) instead of silently ignoring them
        self.update_with_limits(doc, accumulator, usize::MAX, usize::MAX)
    }

    /// Update state with explicit limits for $push and $addToSet
    ///
    /// # OOM Protection
    /// - Returns Err if $push exceeds max_push_elements
    /// - Returns Err if $addToSet exceeds max_addtoset_elements
    /// - Other accumulators are O(1) memory and don't need limits
    pub(crate) fn update_with_limits(
        &mut self,
        doc: &Value,
        accumulator: &Accumulator,
        max_push_elements: usize,
        max_addtoset_elements: usize,
    ) -> Result<()> {
        match (self, accumulator) {
            (
                AccumulatorState::Sum {
                    int_sum,
                    float_sum,
                    has_float,
                    is_count,
                },
                Accumulator::Sum(expr),
            ) => match expr {
                SumExpression::Constant(n) => {
                    *int_sum = int_sum.saturating_add(*n);
                    *is_count = true;
                }
                SumExpression::Field(field) => {
                    if let Some(value) = get_nested_value(doc, field) {
                        if let Some(n) = value.as_i64() {
                            *int_sum = int_sum.saturating_add(n);
                        } else if let Some(f) = value.as_f64() {
                            *float_sum += f;
                            *has_float = true;
                        }
                    }
                }
            },

            (AccumulatorState::Avg { sum, count }, Accumulator::Avg(field)) => {
                if let Some(value) = get_nested_value(doc, field) {
                    if let Some(n) = value.as_f64() {
                        *sum += n;
                        *count = count.saturating_add(1);
                    } else if let Some(n) = value.as_i64() {
                        *sum += n as f64;
                        *count = count.saturating_add(1);
                    }
                }
            }

            (AccumulatorState::Min { value: min_val }, Accumulator::Min(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match min_val {
                        None => *min_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current) == Some(std::cmp::Ordering::Less) {
                                *min_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (AccumulatorState::Max { value: max_val }, Accumulator::Max(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match max_val {
                        None => *max_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current)
                                == Some(std::cmp::Ordering::Greater)
                            {
                                *max_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (
                AccumulatorState::First {
                    value: first_val,
                    captured,
                },
                Accumulator::First(expr),
            ) => {
                // Only capture first document's value (even if missing/null)
                if !*captured {
                    *captured = true;
                    // Store the value (or None if field is missing)
                    *first_val = evaluate_value_expr(doc, expr);
                }
            }

            (
                AccumulatorState::Last {
                    value: last_val,
                    doc_count,
                },
                Accumulator::Last(expr),
            ) => {
                // Always update to latest document's value (even if missing/null)
                *doc_count += 1;
                // Store the value (or None if field is missing)
                *last_val = evaluate_value_expr(doc, expr);
            }

            (AccumulatorState::Push { values }, Accumulator::Push(expr)) => {
                // OOM Protection: check limit BEFORE push
                if values.len() >= max_push_elements {
                    return Err(IronBaseError::AggregationError(format!(
                        "$push exceeded limit: {} elements (max: {}). \
                         Consider using $slice in $project or filtering with $match first.",
                        values.len(),
                        max_push_elements
                    )));
                }
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    // try_reserve for safety
                    values.try_reserve(1).map_err(|_| {
                        IronBaseError::OutOfMemory(format!(
                            "$push failed to allocate memory for element {} in field '{}'. \
                             Consider reducing input documents or increasing system memory.",
                            values.len(),
                            expr_name
                        ))
                    })?;
                    values.push(doc_val.clone());
                }
            }

            (AccumulatorState::AddToSet { seen, values }, Accumulator::AddToSet(expr)) => {
                // OOM Protection: check limit BEFORE insert
                if values.len() >= max_addtoset_elements {
                    return Err(IronBaseError::AggregationError(format!(
                        "$addToSet exceeded limit: {} unique elements (max: {}). \
                         High-cardinality field detected. Consider using $match to filter first.",
                        values.len(),
                        max_addtoset_elements
                    )));
                }
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    let key = canonical_json_string(&doc_val);
                    if seen.insert(key) {
                        // try_reserve for safety
                        values.try_reserve(1).map_err(|_| {
                            IronBaseError::OutOfMemory(format!(
                                "$addToSet failed to allocate memory for element {} in field '{}'. \
                                 Consider reducing input documents or increasing system memory.",
                                values.len(),
                                expr_name
                            ))
                        })?;
                        values.push(doc_val.clone());
                    }
                }
            }

            _ => {
                // Mismatched state/accumulator - should never happen
                debug_assert!(false, "Mismatched AccumulatorState and Accumulator types");
            }
        }
        Ok(())
    }

    /// Update state with explicit limits and return how many elements were added
    ///
    /// FIX (2026-01-25): This variant returns (push_added, addtoset_added) counts
    /// so the caller can track GLOBAL limits across all groups.
    ///
    /// # Returns
    /// * `Ok((push_added, addtoset_added))` - Number of elements added to each accumulator type
    /// * `Err` - If per-group limit is exceeded
    pub(crate) fn update_with_limits_tracking(
        &mut self,
        doc: &Value,
        accumulator: &Accumulator,
        max_push_elements: usize,
        max_addtoset_elements: usize,
    ) -> Result<(usize, usize)> {
        let mut push_added: usize = 0;
        let mut addtoset_added: usize = 0;

        match (self, accumulator) {
            // Non-array accumulators - delegate to update_with_limits, return (0, 0)
            (
                AccumulatorState::Sum {
                    int_sum,
                    float_sum,
                    has_float,
                    is_count,
                },
                Accumulator::Sum(expr),
            ) => match expr {
                SumExpression::Constant(n) => {
                    *int_sum = int_sum.saturating_add(*n);
                    *is_count = true;
                }
                SumExpression::Field(field) => {
                    if let Some(value) = get_nested_value(doc, field) {
                        if let Some(n) = value.as_i64() {
                            *int_sum = int_sum.saturating_add(n);
                        } else if let Some(f) = value.as_f64() {
                            *float_sum += f;
                            *has_float = true;
                        }
                    }
                }
            },

            (AccumulatorState::Avg { sum, count }, Accumulator::Avg(field)) => {
                if let Some(value) = get_nested_value(doc, field) {
                    if let Some(n) = value.as_f64() {
                        *sum += n;
                        *count = count.saturating_add(1);
                    } else if let Some(n) = value.as_i64() {
                        *sum += n as f64;
                        *count = count.saturating_add(1);
                    }
                }
            }

            (AccumulatorState::Min { value: min_val }, Accumulator::Min(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match min_val {
                        None => *min_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current) == Some(std::cmp::Ordering::Less) {
                                *min_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (AccumulatorState::Max { value: max_val }, Accumulator::Max(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match max_val {
                        None => *max_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current)
                                == Some(std::cmp::Ordering::Greater)
                            {
                                *max_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (
                AccumulatorState::First {
                    value: first_val,
                    captured,
                },
                Accumulator::First(expr),
            ) => {
                if !*captured {
                    *captured = true;
                    *first_val = evaluate_value_expr(doc, expr);
                }
            }

            (
                AccumulatorState::Last {
                    value: last_val,
                    doc_count,
                },
                Accumulator::Last(expr),
            ) => {
                *doc_count += 1;
                *last_val = evaluate_value_expr(doc, expr);
            }

            // $push - track how many elements were added
            (AccumulatorState::Push { values }, Accumulator::Push(expr)) => {
                if values.len() >= max_push_elements {
                    return Err(IronBaseError::AggregationError(format!(
                        "$push exceeded per-group limit: {} elements (max: {}). \
                         Consider using $slice in $project or filtering with $match first.",
                        values.len(),
                        max_push_elements
                    )));
                }
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    values.try_reserve(1).map_err(|_| {
                        IronBaseError::OutOfMemory(format!(
                            "$push failed to allocate memory for element {} in field '{}'.",
                            values.len(),
                            expr_name
                        ))
                    })?;
                    values.push(doc_val.clone());
                    push_added = 1; // Track that we added 1 element
                }
            }

            // $addToSet - track how many unique elements were added
            (AccumulatorState::AddToSet { seen, values }, Accumulator::AddToSet(expr)) => {
                if values.len() >= max_addtoset_elements {
                    return Err(IronBaseError::AggregationError(format!(
                        "$addToSet exceeded per-group limit: {} unique elements (max: {}). \
                         High-cardinality field detected. Consider using $match to filter first.",
                        values.len(),
                        max_addtoset_elements
                    )));
                }
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    let key = canonical_json_string(&doc_val);
                    if seen.insert(key) {
                        values.try_reserve(1).map_err(|_| {
                            IronBaseError::OutOfMemory(format!(
                                "$addToSet failed to allocate memory for element {} in field '{}'.",
                                values.len(),
                                expr_name
                            ))
                        })?;
                        values.push(doc_val.clone());
                        addtoset_added = 1; // Track that we added 1 unique element
                    }
                }
            }

            _ => {
                debug_assert!(false, "Mismatched AccumulatorState and Accumulator types");
            }
        }

        Ok((push_added, addtoset_added))
    }

    /// Update state using centralized AggregationLimitContext
    ///
    /// This method uses the context for per-group tracking of $push/$addToSet.
    /// The context automatically tracks counts and provides consistent error messages.
    ///
    /// # Arguments
    /// * `doc` - Document to process
    /// * `accumulator` - Accumulator definition
    /// * `group_hash` - Hash of the group key (for per-group tracking)
    /// * `ctx` - Shared context for limit tracking
    #[allow(dead_code)] // Will be used in pipeline.rs integration
    pub(crate) fn update_with_context(
        &mut self,
        doc: &Value,
        accumulator: &Accumulator,
        group_hash: u64,
        ctx: &AggregationLimitContext,
    ) -> Result<()> {
        match (self, accumulator) {
            // Sum, Avg, Min, Max, First, Last - O(1) memory, no limit needed
            (
                AccumulatorState::Sum {
                    int_sum,
                    float_sum,
                    has_float,
                    is_count,
                },
                Accumulator::Sum(expr),
            ) => match expr {
                SumExpression::Constant(n) => {
                    *int_sum = int_sum.saturating_add(*n);
                    *is_count = true;
                }
                SumExpression::Field(field) => {
                    if let Some(value) = get_nested_value(doc, field) {
                        if let Some(n) = value.as_i64() {
                            *int_sum = int_sum.saturating_add(n);
                        } else if let Some(f) = value.as_f64() {
                            *float_sum += f;
                            *has_float = true;
                        }
                    }
                }
            },

            (AccumulatorState::Avg { sum, count }, Accumulator::Avg(field)) => {
                if let Some(value) = get_nested_value(doc, field) {
                    if let Some(n) = value.as_f64() {
                        *sum += n;
                        *count = count.saturating_add(1);
                    } else if let Some(n) = value.as_i64() {
                        *sum += n as f64;
                        *count = count.saturating_add(1);
                    }
                }
            }

            (AccumulatorState::Min { value: min_val }, Accumulator::Min(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match min_val {
                        None => *min_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current) == Some(std::cmp::Ordering::Less) {
                                *min_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (AccumulatorState::Max { value: max_val }, Accumulator::Max(expr)) => {
                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    match max_val {
                        None => *max_val = Some(doc_val.clone()),
                        Some(current) => {
                            if compare_values(&doc_val, current)
                                == Some(std::cmp::Ordering::Greater)
                            {
                                *max_val = Some(doc_val.clone());
                            }
                        }
                    }
                }
            }

            (
                AccumulatorState::First {
                    value: first_val,
                    captured,
                },
                Accumulator::First(expr),
            ) => {
                if !*captured {
                    *captured = true;
                    *first_val = evaluate_value_expr(doc, expr);
                }
            }

            (
                AccumulatorState::Last {
                    value: last_val,
                    doc_count,
                },
                Accumulator::Last(expr),
            ) => {
                *doc_count += 1;
                *last_val = evaluate_value_expr(doc, expr);
            }

            // $push - uses context for per-group limit tracking
            (AccumulatorState::Push { values }, Accumulator::Push(expr)) => {
                // Check limit via context BEFORE push
                ctx.check_push_limit(group_hash)?;

                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    values.try_reserve(1).map_err(|_| {
                        IronBaseError::OutOfMemory(format!(
                            "$push failed to allocate memory for element {} in field '{}'. \
                             Consider reducing input documents or increasing system memory.",
                            values.len(),
                            expr_name
                        ))
                    })?;
                    values.push(doc_val.clone());

                    // Increment counter in context
                    ctx.increment_push(group_hash)?;
                }
            }

            // $addToSet - uses context for per-group limit tracking
            (AccumulatorState::AddToSet { seen, values }, Accumulator::AddToSet(expr)) => {
                // Check limit via context BEFORE insert
                ctx.check_addtoset_limit(group_hash)?;

                if let Some(doc_val) = evaluate_value_expr(doc, expr) {
                    let expr_name = expr_debug_name(expr);
                    let key = canonical_json_string(&doc_val);
                    if seen.insert(key) {
                        values.try_reserve(1).map_err(|_| {
                            IronBaseError::OutOfMemory(format!(
                                "$addToSet failed to allocate memory for element {} in field '{}'. \
                                 Consider reducing input documents or increasing system memory.",
                                values.len(),
                                expr_name
                            ))
                        })?;
                        values.push(doc_val.clone());

                        // Increment counter in context (only for new elements)
                        ctx.increment_addtoset(group_hash)?;
                    }
                }
            }

            _ => {
                debug_assert!(false, "Mismatched AccumulatorState and Accumulator types");
            }
        }
        Ok(())
    }

    /// Finalize state to produce the output value
    pub(crate) fn finalize(self) -> Value {
        match self {
            AccumulatorState::Sum {
                int_sum,
                float_sum,
                has_float,
                ..
            } => {
                if has_float {
                    Value::from(float_sum + int_sum as f64)
                } else {
                    Value::from(int_sum)
                }
            }

            AccumulatorState::Avg { sum, count } => {
                if count > 0 {
                    Value::from(sum / count as f64)
                } else {
                    Value::Null
                }
            }

            AccumulatorState::Min { value } => value.unwrap_or(Value::Null),

            AccumulatorState::Max { value } => value.unwrap_or(Value::Null),

            AccumulatorState::First { value, .. } => value.unwrap_or(Value::Null),

            AccumulatorState::Last { value, .. } => value.unwrap_or(Value::Null),

            AccumulatorState::Push { values } => Value::Array(values),

            AccumulatorState::AddToSet { values, .. } => Value::Array(values),
        }
    }
}
