// src/aggregation/stages/project_stage.rs
// $project stage implementation

use crate::aggregation::types::{
    ProjectExpression, ProjectField, ProjectStage, ReduceExpression, ReduceInExpr,
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
            _ => Err(IronBaseError::AggregationError(format!(
                "Unknown projection expression operator: {}",
                op
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
            let projected = self.project_document(&doc)?;
            results.push(projected);
        }

        Ok(results)
    }

    fn project_document(&self, doc: &Value) -> Result<Value> {
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
