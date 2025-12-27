// src/aggregation/stages/project_stage.rs
// $project stage implementation

use crate::aggregation::types::{
    ArithmeticOperand, ProjectExpression, ProjectField, ProjectStage, ReduceExpression,
    ReduceInExpr,
};
use crate::error::{IronBaseError, Result};
use crate::value_utils::get_nested_value;
use serde_json::Value;
use std::collections::HashMap;

impl ProjectStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let mut fields = HashMap::new();

            for (field, value) in obj {
                let project_field = if let Some(n) = value.as_i64() {
                    match n {
                        1 => ProjectField::Include,
                        0 => ProjectField::Exclude,
                        _ => {
                            return Err(IronBaseError::AggregationError(format!(
                                "Invalid project value: {}",
                                n
                            )))
                        }
                    }
                } else if let Some(s) = value.as_str() {
                    if s.starts_with('$') {
                        ProjectField::Rename(s.to_string())
                    } else {
                        return Err(IronBaseError::AggregationError(format!(
                            "Invalid project expression: {}",
                            s
                        )));
                    }
                } else if let Value::Object(expr_obj) = value {
                    Self::parse_expression(expr_obj)?
                } else {
                    return Err(IronBaseError::AggregationError(
                        "Project field must be 0, 1, field reference, or expression object"
                            .to_string(),
                    ));
                };

                fields.insert(field.clone(), project_field);
            }

            Ok(ProjectStage { fields })
        } else {
            Err(IronBaseError::AggregationError(
                "$project must be an object".to_string(),
            ))
        }
    }

    fn parse_expression(obj: &serde_json::Map<String, Value>) -> Result<ProjectField> {
        if obj.len() != 1 {
            return Err(IronBaseError::AggregationError(
                "Expression object must have exactly one operator".to_string(),
            ));
        }

        let (op, arg) = obj.iter().next().unwrap();

        match op.as_str() {
            "$size" => {
                if let Some(field_ref) = arg.as_str() {
                    if field_ref.starts_with('$') {
                        let field_name = field_ref.trim_start_matches('$').to_string();
                        Ok(ProjectField::Expression(ProjectExpression::Size(
                            field_name,
                        )))
                    } else {
                        Err(IronBaseError::AggregationError(
                            "$size argument must be a field reference starting with $".to_string(),
                        ))
                    }
                } else {
                    Err(IronBaseError::AggregationError(
                        "$size argument must be a string field reference".to_string(),
                    ))
                }
            }
            "$reduce" => Self::parse_reduce_expression(arg),
            // Arithmetic operators
            "$add" => Self::parse_variadic_arithmetic(arg, ProjectExpression::Add),
            "$multiply" => Self::parse_variadic_arithmetic(arg, ProjectExpression::Multiply),
            "$subtract" => Self::parse_binary_arithmetic(arg, "$subtract", |a, b| {
                ProjectExpression::Subtract(Box::new(a), Box::new(b))
            }),
            "$divide" => Self::parse_binary_arithmetic(arg, "$divide", |a, b| {
                ProjectExpression::Divide(Box::new(a), Box::new(b))
            }),
            "$mod" => Self::parse_binary_arithmetic(arg, "$mod", |a, b| {
                ProjectExpression::Mod(Box::new(a), Box::new(b))
            }),
            "$abs" => {
                Self::parse_unary_arithmetic(arg, "$abs", |a| ProjectExpression::Abs(Box::new(a)))
            }
            "$ceil" => {
                Self::parse_unary_arithmetic(arg, "$ceil", |a| ProjectExpression::Ceil(Box::new(a)))
            }
            "$floor" => Self::parse_unary_arithmetic(arg, "$floor", |a| {
                ProjectExpression::Floor(Box::new(a))
            }),
            "$round" => Self::parse_round_expression(arg),
            _ => Err(IronBaseError::AggregationError(format!(
                "Unknown projection expression operator: {}",
                op
            ))),
        }
    }

    /// Parse a variadic arithmetic expression like $add or $multiply
    fn parse_variadic_arithmetic<F>(arg: &Value, constructor: F) -> Result<ProjectField>
    where
        F: FnOnce(Vec<ArithmeticOperand>) -> ProjectExpression,
    {
        let arr = arg.as_array().ok_or_else(|| {
            IronBaseError::AggregationError("Arithmetic operator requires an array".to_string())
        })?;

        if arr.len() < 2 {
            return Err(IronBaseError::AggregationError(
                "Arithmetic operator requires at least 2 operands".to_string(),
            ));
        }

        let operands: Result<Vec<ArithmeticOperand>> =
            arr.iter().map(Self::parse_arithmetic_operand).collect();

        Ok(ProjectField::Expression(constructor(operands?)))
    }

    /// Parse a binary arithmetic expression like $subtract or $divide
    fn parse_binary_arithmetic<F>(
        arg: &Value,
        op_name: &str,
        constructor: F,
    ) -> Result<ProjectField>
    where
        F: FnOnce(ArithmeticOperand, ArithmeticOperand) -> ProjectExpression,
    {
        let arr = arg.as_array().ok_or_else(|| {
            IronBaseError::AggregationError(format!("{} requires an array", op_name))
        })?;

        if arr.len() != 2 {
            return Err(IronBaseError::AggregationError(format!(
                "{} requires exactly 2 operands",
                op_name
            )));
        }

        let left = Self::parse_arithmetic_operand(&arr[0])?;
        let right = Self::parse_arithmetic_operand(&arr[1])?;

        Ok(ProjectField::Expression(constructor(left, right)))
    }

    /// Parse a unary arithmetic expression like $abs, $ceil, $floor
    fn parse_unary_arithmetic<F>(arg: &Value, op_name: &str, constructor: F) -> Result<ProjectField>
    where
        F: FnOnce(ArithmeticOperand) -> ProjectExpression,
    {
        // Can be a direct operand or a single-element array
        let operand = if let Some(arr) = arg.as_array() {
            if arr.len() != 1 {
                return Err(IronBaseError::AggregationError(format!(
                    "{} requires exactly 1 operand",
                    op_name
                )));
            }
            Self::parse_arithmetic_operand(&arr[0])?
        } else {
            Self::parse_arithmetic_operand(arg)?
        };

        Ok(ProjectField::Expression(constructor(operand)))
    }

    /// Parse $round expression: { $round: ["$value", 2] } or { $round: "$value" }
    fn parse_round_expression(arg: &Value) -> Result<ProjectField> {
        if let Some(arr) = arg.as_array() {
            if arr.is_empty() || arr.len() > 2 {
                return Err(IronBaseError::AggregationError(
                    "$round requires 1 or 2 operands".to_string(),
                ));
            }
            let operand = Self::parse_arithmetic_operand(&arr[0])?;
            let precision = if arr.len() == 2 {
                arr[1].as_i64().map(|n| n as i32)
            } else {
                None
            };
            Ok(ProjectField::Expression(ProjectExpression::Round(
                Box::new(operand),
                precision,
            )))
        } else {
            let operand = Self::parse_arithmetic_operand(arg)?;
            Ok(ProjectField::Expression(ProjectExpression::Round(
                Box::new(operand),
                None,
            )))
        }
    }

    /// Parse an arithmetic operand: field reference, literal, or nested expression
    fn parse_arithmetic_operand(value: &Value) -> Result<ArithmeticOperand> {
        match value {
            Value::String(s) if s.starts_with('$') => {
                Ok(ArithmeticOperand::Field(s[1..].to_string()))
            }
            Value::Number(n) => {
                let num = n.as_f64().ok_or_else(|| {
                    IronBaseError::AggregationError("Invalid number in expression".to_string())
                })?;
                Ok(ArithmeticOperand::Literal(num))
            }
            Value::Object(obj) if obj.len() == 1 => {
                // Nested expression like { "$add": [...] }
                let field = Self::parse_expression(obj)?;
                if let ProjectField::Expression(expr) = field {
                    Ok(ArithmeticOperand::Expression(Box::new(expr)))
                } else {
                    Err(IronBaseError::AggregationError(
                        "Expected expression in nested operand".to_string(),
                    ))
                }
            }
            _ => Err(IronBaseError::AggregationError(format!(
                "Invalid arithmetic operand: {:?}",
                value
            ))),
        }
    }

    fn parse_reduce_expression(spec: &Value) -> Result<ProjectField> {
        let obj = spec.as_object().ok_or_else(|| {
            IronBaseError::AggregationError("$reduce must be an object".to_string())
        })?;

        let input = obj.get("input").and_then(|v| v.as_str()).ok_or_else(|| {
            IronBaseError::AggregationError("$reduce requires 'input' field reference".to_string())
        })?;

        if !input.starts_with('$') {
            return Err(IronBaseError::AggregationError(
                "$reduce input must be a field reference starting with $".to_string(),
            ));
        }

        let input_field = input.trim_start_matches('$').to_string();

        let initial_value = obj.get("initialValue").cloned().ok_or_else(|| {
            IronBaseError::AggregationError("$reduce requires 'initialValue'".to_string())
        })?;

        let in_expr = obj.get("in").ok_or_else(|| {
            IronBaseError::AggregationError("$reduce requires 'in' expression".to_string())
        })?;

        let reduce_in = Self::parse_reduce_in_expr(in_expr)?;

        Ok(ProjectField::Expression(ProjectExpression::Reduce(
            ReduceExpression {
                input: input_field,
                initial_value,
                in_expr: reduce_in,
            },
        )))
    }

    fn parse_reduce_in_expr(expr: &Value) -> Result<ReduceInExpr> {
        let obj = expr.as_object().ok_or_else(|| {
            IronBaseError::AggregationError("$reduce 'in' must be an expression object".to_string())
        })?;

        if obj.len() != 1 {
            return Err(IronBaseError::AggregationError(
                "$reduce 'in' must have exactly one operator".to_string(),
            ));
        }

        let (op, args) = obj.iter().next().unwrap();
        let this_field = Self::parse_this_field_reference(args);

        match op.as_str() {
            "$add" => {
                Self::validate_reduce_args(args, "$add")?;
                match this_field {
                    Some(field) => Ok(ReduceInExpr::AddField(field)),
                    None => Ok(ReduceInExpr::Add),
                }
            }
            "$multiply" => {
                Self::validate_reduce_args(args, "$multiply")?;
                match this_field {
                    Some(field) => Ok(ReduceInExpr::MultiplyField(field)),
                    None => Ok(ReduceInExpr::Multiply),
                }
            }
            "$concat" => {
                if let Some(arr) = args.as_array() {
                    if arr.len() == 3 {
                        if let Some(sep) = arr.get(1).and_then(|v| v.as_str()) {
                            if !sep.starts_with("$$") {
                                match this_field {
                                    Some(field) => {
                                        return Ok(ReduceInExpr::ConcatFieldWithSeparator {
                                            field,
                                            separator: sep.to_string(),
                                        })
                                    }
                                    None => {
                                        return Ok(ReduceInExpr::ConcatWithSeparator(
                                            sep.to_string(),
                                        ))
                                    }
                                }
                            }
                        }
                    }
                }
                Self::validate_reduce_args(args, "$concat")?;
                match this_field {
                    Some(field) => Ok(ReduceInExpr::ConcatField(field)),
                    None => Ok(ReduceInExpr::Concat),
                }
            }
            _ => Err(IronBaseError::AggregationError(format!(
                "Unsupported $reduce operator: {}. Supported: $add, $multiply, $concat",
                op
            ))),
        }
    }

    fn parse_this_field_reference(args: &Value) -> Option<String> {
        if let Some(arr) = args.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if s.starts_with("$$this.") {
                        return Some(s.trim_start_matches("$$this.").to_string());
                    }
                }
            }
        }
        None
    }

    fn validate_reduce_args(args: &Value, op_name: &str) -> Result<()> {
        let arr = args.as_array().ok_or_else(|| {
            IronBaseError::AggregationError(format!("{} arguments must be an array", op_name))
        })?;

        let has_value = arr.iter().any(|v| v.as_str() == Some("$$value"));
        let has_this = arr.iter().any(|v| {
            v.as_str()
                .map(|s| s == "$$this" || s.starts_with("$$this."))
                .unwrap_or(false)
        });

        if !has_value || !has_this {
            return Err(IronBaseError::AggregationError(format!(
                "{} in $reduce must use $$value and $$this (or $$this.field)",
                op_name
            )));
        }

        Ok(())
    }

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let projected = self.project_one(&doc)?;
            results.push(projected);
        }

        Ok(results)
    }

    /// Project a single document (for streaming execution)
    ///
    /// Used by the streaming pipeline to transform documents one at a time
    /// without loading the entire collection into memory.
    pub(crate) fn project_one(&self, doc: &Value) -> Result<Value> {
        let mut result = serde_json::Map::new();

        if let Value::Object(obj) = doc {
            let has_inclusions = self.fields.values().any(|f| {
                matches!(
                    f,
                    ProjectField::Include | ProjectField::Rename(_) | ProjectField::Expression(_)
                )
            });
            let has_non_id_exclusions = self
                .fields
                .iter()
                .any(|(field, action)| matches!(action, ProjectField::Exclude) && field != "_id");

            let include_mode = has_inclusions && !has_non_id_exclusions;

            if include_mode {
                for (field, action) in &self.fields {
                    match action {
                        ProjectField::Include => {
                            if let Some(value) = get_nested_value(doc, field) {
                                result.insert(field.clone(), value.clone());
                            }
                        }
                        ProjectField::Rename(source) => {
                            let source_field = source.trim_start_matches('$');
                            if let Some(value) = get_nested_value(doc, source_field) {
                                result.insert(field.clone(), value.clone());
                            }
                        }
                        ProjectField::Expression(expr) => {
                            let value = Self::evaluate_expression(expr, doc);
                            result.insert(field.clone(), value);
                        }
                        ProjectField::Exclude => {}
                    }
                }
            } else {
                for (field, value) in obj {
                    if let Some(action) = self.fields.get(field) {
                        match action {
                            ProjectField::Exclude => {}
                            ProjectField::Include => {
                                result.insert(field.clone(), value.clone());
                            }
                            ProjectField::Rename(_) | ProjectField::Expression(_) => {}
                        }
                    } else {
                        result.insert(field.clone(), value.clone());
                    }
                }

                for (target_field, action) in &self.fields {
                    match action {
                        ProjectField::Rename(source) => {
                            let source_field = source.trim_start_matches('$');
                            if let Some(value) = get_nested_value(doc, source_field) {
                                result.insert(target_field.clone(), value.clone());
                            }
                        }
                        ProjectField::Expression(expr) => {
                            let value = Self::evaluate_expression(expr, doc);
                            result.insert(target_field.clone(), value);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(Value::Object(result))
    }

    fn evaluate_expression(expr: &ProjectExpression, doc: &Value) -> Value {
        match expr {
            ProjectExpression::Size(field_name) => {
                if let Some(Value::Array(arr)) = get_nested_value(doc, field_name) {
                    Value::Number(serde_json::Number::from(arr.len()))
                } else {
                    Value::Null
                }
            }
            ProjectExpression::Reduce(reduce_expr) => Self::evaluate_reduce(reduce_expr, doc),
            ProjectExpression::Add(operands) => {
                let mut result = 0.0;
                for op in operands {
                    if let Some(val) = Self::evaluate_operand(op, doc) {
                        result += val;
                    } else {
                        return Value::Null;
                    }
                }
                Value::from(result)
            }
            ProjectExpression::Multiply(operands) => {
                let mut result = 1.0;
                for op in operands {
                    if let Some(val) = Self::evaluate_operand(op, doc) {
                        result *= val;
                    } else {
                        return Value::Null;
                    }
                }
                Value::from(result)
            }
            ProjectExpression::Subtract(left, right) => {
                match (
                    Self::evaluate_operand(left, doc),
                    Self::evaluate_operand(right, doc),
                ) {
                    (Some(l), Some(r)) => Value::from(l - r),
                    _ => Value::Null,
                }
            }
            ProjectExpression::Divide(left, right) => {
                match (
                    Self::evaluate_operand(left, doc),
                    Self::evaluate_operand(right, doc),
                ) {
                    (Some(l), Some(r)) if r != 0.0 => Value::from(l / r),
                    (Some(_), Some(_)) => Value::Null, // Division by zero
                    _ => Value::Null,
                }
            }
            ProjectExpression::Mod(left, right) => {
                match (
                    Self::evaluate_operand(left, doc),
                    Self::evaluate_operand(right, doc),
                ) {
                    (Some(l), Some(r)) if r != 0.0 => Value::from(l % r),
                    (Some(_), Some(_)) => Value::Null, // Mod by zero
                    _ => Value::Null,
                }
            }
            ProjectExpression::Abs(operand) => Self::evaluate_operand(operand, doc)
                .map(|v| Value::from(v.abs()))
                .unwrap_or(Value::Null),
            ProjectExpression::Ceil(operand) => Self::evaluate_operand(operand, doc)
                .map(|v| Value::from(v.ceil()))
                .unwrap_or(Value::Null),
            ProjectExpression::Floor(operand) => Self::evaluate_operand(operand, doc)
                .map(|v| Value::from(v.floor()))
                .unwrap_or(Value::Null),
            ProjectExpression::Round(operand, precision) => Self::evaluate_operand(operand, doc)
                .map(|v| {
                    let p = precision.unwrap_or(0);
                    let multiplier = 10_f64.powi(p);
                    Value::from((v * multiplier).round() / multiplier)
                })
                .unwrap_or(Value::Null),
        }
    }

    /// Evaluate an arithmetic operand to a numeric value
    fn evaluate_operand(op: &ArithmeticOperand, doc: &Value) -> Option<f64> {
        match op {
            ArithmeticOperand::Field(field) => {
                get_nested_value(doc, field).and_then(Self::value_to_f64_opt)
            }
            ArithmeticOperand::Literal(n) => Some(*n),
            ArithmeticOperand::Expression(expr) => {
                let result = Self::evaluate_expression(expr, doc);
                Self::value_to_f64_opt(&result)
            }
        }
    }

    /// Convert Value to f64, returning None for non-numeric values
    fn value_to_f64_opt(value: &Value) -> Option<f64> {
        match value {
            Value::Number(n) => n.as_f64(),
            _ => None,
        }
    }

    fn evaluate_reduce(expr: &ReduceExpression, doc: &Value) -> Value {
        let array = match get_nested_value(doc, &expr.input) {
            Some(Value::Array(arr)) => arr.clone(),
            _ => return Value::Null,
        };

        let mut accumulator = expr.initial_value.clone();

        for element in array {
            accumulator = match &expr.in_expr {
                ReduceInExpr::Add => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    let elem_num = Self::value_to_f64(&element);
                    Value::from(acc_num + elem_num)
                }
                ReduceInExpr::AddField(field) => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    let elem_value = get_nested_value(&element, field).unwrap_or(&Value::Null);
                    let elem_num = Self::value_to_f64(elem_value);
                    Value::from(acc_num + elem_num)
                }
                ReduceInExpr::Multiply => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    let elem_num = Self::value_to_f64(&element);
                    Value::from(acc_num * elem_num)
                }
                ReduceInExpr::MultiplyField(field) => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    let elem_value = get_nested_value(&element, field).unwrap_or(&Value::Null);
                    let elem_num = Self::value_to_f64(elem_value);
                    Value::from(acc_num * elem_num)
                }
                ReduceInExpr::Concat => {
                    let acc_str = Self::value_to_string(&accumulator);
                    let elem_str = Self::value_to_string(&element);
                    Value::from(format!("{}{}", acc_str, elem_str))
                }
                ReduceInExpr::ConcatField(field) => {
                    let acc_str = Self::value_to_string(&accumulator);
                    let elem_value = get_nested_value(&element, field).unwrap_or(&Value::Null);
                    let elem_str = Self::value_to_string(elem_value);
                    Value::from(format!("{}{}", acc_str, elem_str))
                }
                ReduceInExpr::ConcatWithSeparator(sep) => {
                    let acc_str = Self::value_to_string(&accumulator);
                    let elem_str = Self::value_to_string(&element);
                    if acc_str.is_empty() {
                        Value::from(elem_str)
                    } else {
                        Value::from(format!("{}{}{}", acc_str, sep, elem_str))
                    }
                }
                ReduceInExpr::ConcatFieldWithSeparator { field, separator } => {
                    let acc_str = Self::value_to_string(&accumulator);
                    let elem_value = get_nested_value(&element, field).unwrap_or(&Value::Null);
                    let elem_str = Self::value_to_string(elem_value);
                    if acc_str.is_empty() {
                        Value::from(elem_str)
                    } else {
                        Value::from(format!("{}{}{}", acc_str, separator, elem_str))
                    }
                }
            };
        }

        accumulator
    }

    fn value_to_f64(value: &Value) -> f64 {
        match value {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    fn value_to_string(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => String::new(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_project_multiply() {
        let stage = ProjectStage::from_json(&json!({
            "total": {"$multiply": ["$price", "$qty"]}
        }))
        .unwrap();

        let doc = json!({"price": 10, "qty": 5});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["total"], json!(50.0));
    }

    #[test]
    fn test_project_add() {
        let stage = ProjectStage::from_json(&json!({
            "sum": {"$add": ["$a", "$b", "$c"]}
        }))
        .unwrap();

        let doc = json!({"a": 10, "b": 20, "c": 30});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["sum"], json!(60.0));
    }

    #[test]
    fn test_project_subtract() {
        let stage = ProjectStage::from_json(&json!({
            "diff": {"$subtract": ["$total", "$discount"]}
        }))
        .unwrap();

        let doc = json!({"total": 100, "discount": 15});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["diff"], json!(85.0));
    }

    #[test]
    fn test_project_divide() {
        let stage = ProjectStage::from_json(&json!({
            "avg": {"$divide": ["$sum", "$count"]}
        }))
        .unwrap();

        let doc = json!({"sum": 150, "count": 3});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["avg"], json!(50.0));
    }

    #[test]
    fn test_project_divide_by_zero() {
        let stage = ProjectStage::from_json(&json!({
            "result": {"$divide": ["$a", "$b"]}
        }))
        .unwrap();

        let doc = json!({"a": 10, "b": 0});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["result"], Value::Null);
    }

    #[test]
    fn test_project_nested_expression() {
        // { $multiply: [{ $add: ["$price", "$tax"] }, "$qty"] }
        let stage = ProjectStage::from_json(&json!({
            "total": {"$multiply": [{"$add": ["$price", "$tax"]}, "$qty"]}
        }))
        .unwrap();

        let doc = json!({"price": 100, "tax": 10, "qty": 2});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["total"], json!(220.0)); // (100+10) * 2
    }

    #[test]
    fn test_project_abs() {
        let stage = ProjectStage::from_json(&json!({
            "absVal": {"$abs": "$diff"}
        }))
        .unwrap();

        let doc = json!({"diff": -42});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["absVal"], json!(42.0));
    }

    #[test]
    fn test_project_ceil_floor() {
        let stage = ProjectStage::from_json(&json!({
            "ceiling": {"$ceil": "$val"},
            "flooring": {"$floor": "$val"}
        }))
        .unwrap();

        let doc = json!({"val": 3.7});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["ceiling"], json!(4.0));
        assert_eq!(result[0]["flooring"], json!(3.0));
    }

    #[test]
    fn test_project_round() {
        let stage = ProjectStage::from_json(&json!({
            "rounded": {"$round": ["$val", 2]}
        }))
        .unwrap();

        let doc = json!({"val": 3.14159});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["rounded"], json!(3.14));
    }

    #[test]
    fn test_project_mod() {
        let stage = ProjectStage::from_json(&json!({
            "remainder": {"$mod": ["$value", 3]}
        }))
        .unwrap();

        let doc = json!({"value": 17});
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["remainder"], json!(2.0));
    }

    #[test]
    fn test_project_with_literal() {
        let stage = ProjectStage::from_json(&json!({
            "withTax": {"$multiply": ["$price", 1.1]}
        }))
        .unwrap();

        let doc = json!({"price": 100});
        let result = stage.execute(vec![doc]).unwrap();

        // 100 * 1.1 = 110.0
        let val = result[0]["withTax"].as_f64().unwrap();
        assert!((val - 110.0).abs() < 0.001);
    }

    #[test]
    fn test_project_missing_field_returns_null() {
        let stage = ProjectStage::from_json(&json!({
            "total": {"$multiply": ["$price", "$qty"]}
        }))
        .unwrap();

        let doc = json!({"price": 10}); // qty is missing
        let result = stage.execute(vec![doc]).unwrap();

        assert_eq!(result[0]["total"], Value::Null);
    }
}
