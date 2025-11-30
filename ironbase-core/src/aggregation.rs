// src/aggregation.rs
// Aggregation pipeline implementation

use crate::document::Document;
use crate::error::{MongoLiteError, Result};
use crate::query::Query;
use crate::value_utils::{canonical_json_string, get_nested_value, set_nested_value};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Parse a field reference from JSON value (e.g., "$fieldName" -> "fieldName")
///
/// Used by accumulators like $avg, $min, $max, $first, $last
fn parse_field_reference(value: &Value, op_name: &str) -> Result<String> {
    if let Some(s) = value.as_str() {
        if s.starts_with('$') {
            Ok(s.trim_start_matches('$').to_string())
        } else {
            Err(MongoLiteError::AggregationError(format!(
                "{} field reference must start with $",
                op_name
            )))
        }
    } else {
        Err(MongoLiteError::AggregationError(format!(
            "{} must be a field reference",
            op_name
        )))
    }
}

/// Compute min or max over documents using a comparison function
///
/// Used by $min and $max accumulators
fn compute_extremum<F>(docs: &[Value], field: &str, compare: F) -> Result<Value>
where
    F: Fn(f64, f64) -> f64,
{
    let mut result: Option<f64> = None;

    for doc in docs {
        if let Some(value) = get_nested_value(doc, field) {
            let num = if let Some(n) = value.as_f64() {
                n
            } else if let Some(n) = value.as_i64() {
                n as f64
            } else {
                continue;
            };

            result = Some(result.map_or(num, |r| compare(r, num)));
        }
    }

    Ok(result.map(Value::from).unwrap_or(Value::Null))
}

/// Aggregation pipeline
#[derive(Debug, Clone)]
pub struct Pipeline {
    stages: Vec<Stage>,
}

/// Pipeline stage
#[derive(Debug, Clone)]
pub enum Stage {
    Match(MatchStage),
    Project(ProjectStage),
    Group(GroupStage),
    Sort(SortStage),
    Limit(LimitStage),
    Skip(SkipStage),
    Unwind(UnwindStage),
}

/// $match stage - filter documents
#[derive(Debug, Clone)]
pub struct MatchStage {
    query: Query,
}

/// $project stage - reshape documents
#[derive(Debug, Clone)]
pub struct ProjectStage {
    fields: HashMap<String, ProjectField>,
}

#[derive(Debug, Clone)]
pub enum ProjectField {
    Include,                       // 1
    Exclude,                       // 0
    Rename(String),                // "$fieldName"
    Expression(ProjectExpression), // {"$size": "$field"}, etc.
}

/// Expressions that can be used in $project stage
#[derive(Debug, Clone)]
pub enum ProjectExpression {
    /// $size - returns the length of an array field
    Size(String), // Field name (e.g., "$tags" -> "tags")
    /// $reduce - apply a custom reduction to an array
    Reduce(ReduceExpression),
}

/// $reduce expression - reduces an array to a single value
///
/// # MongoDB Syntax
///
/// ```json
/// {$reduce: {
///     input: "$arrayField",
///     initialValue: 0,
///     in: {$add: ["$$value", "$$this"]}
/// }}
/// ```
///
/// Special variables:
/// - `$$value` - the accumulated value from previous iterations
/// - `$$this` - the current array element
#[derive(Debug, Clone)]
pub struct ReduceExpression {
    /// Input array field name (without $)
    input: String,
    /// Initial value for the accumulator
    initial_value: Value,
    /// Reduction expression to apply
    in_expr: ReduceInExpr,
}

/// Supported reduction operations
#[derive(Debug, Clone)]
pub enum ReduceInExpr {
    /// {$add: ["$$value", "$$this"]} - sum values
    Add,
    /// {$add: ["$$value", "$$this.field"]} - sum field values from objects
    AddField(String),
    /// {$multiply: ["$$value", "$$this"]} - multiply values
    Multiply,
    /// {$multiply: ["$$value", "$$this.field"]} - multiply field values from objects
    MultiplyField(String),
    /// {$concat: ["$$value", "$$this"]} - concatenate strings
    Concat,
    /// {$concat: ["$$value", "$$this.field"]} - concatenate field values from objects
    ConcatField(String),
    /// {$concat: ["$$value", separator, "$$this"]} - concatenate with separator
    ConcatWithSeparator(String),
    /// {$concat: ["$$value", separator, "$$this.field"]} - concat fields with separator
    ConcatFieldWithSeparator { field: String, separator: String },
}

/// $group stage - group documents and compute aggregates
#[derive(Debug, Clone)]
pub struct GroupStage {
    id: GroupId,
    accumulators: HashMap<String, Accumulator>,
}

#[derive(Debug, Clone)]
pub enum GroupId {
    Field(String), // "$city"
    Null,          // null (all documents in one group)
}

#[derive(Debug, Clone)]
pub enum Accumulator {
    Sum(SumExpression),
    Avg(String), // Field name
    Min(String),
    Max(String),
    First(String),
    Last(String),
    Count,
    Push(String),     // $push - collect all values into array
    AddToSet(String), // $addToSet - collect unique values into array
}

#[derive(Debug, Clone)]
pub enum SumExpression {
    Constant(i64), // {"$sum": 1} - count
    Field(String), // {"$sum": "$amount"} - sum field values
}

/// $sort stage - sort documents
#[derive(Debug, Clone)]
pub struct SortStage {
    fields: Vec<(String, SortDirection)>,
}

#[derive(Debug, Clone)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// $limit stage - limit number of documents
#[derive(Debug, Clone)]
pub struct LimitStage {
    limit: usize,
}

/// $skip stage - skip documents
#[derive(Debug, Clone)]
pub struct SkipStage {
    skip: usize,
}

/// $unwind stage - deconstruct an array field
///
/// Outputs one document per array element. The path field in each output document
/// is replaced with the array element value.
///
/// # MongoDB Syntax
///
/// Simple form: `{$unwind: "$arrayField"}`
///
/// Extended form:
/// ```json
/// {$unwind: {
///     path: "$arrayField",
///     includeArrayIndex: "indexField",      // optional
///     preserveNullAndEmptyArrays: true      // optional
/// }}
/// ```
#[derive(Debug, Clone)]
pub struct UnwindStage {
    /// Field path to unwind (without leading $)
    path: String,
    /// Optional field name to store array index
    include_array_index: Option<String>,
    /// If true, preserve documents with null/missing/empty arrays
    preserve_null_and_empty_arrays: bool,
}

impl Pipeline {
    /// Create pipeline from JSON array
    pub fn from_json(pipeline_json: &Value) -> Result<Self> {
        if let Value::Array(stages_array) = pipeline_json {
            if stages_array.is_empty() {
                return Err(MongoLiteError::AggregationError(
                    "Pipeline cannot be empty".to_string(),
                ));
            }

            let mut stages = Vec::new();
            for stage_json in stages_array {
                let stage = Stage::from_json(stage_json)?;
                stages.push(stage);
            }

            Ok(Pipeline { stages })
        } else {
            Err(MongoLiteError::AggregationError(
                "Pipeline must be an array".to_string(),
            ))
        }
    }

    /// Execute pipeline on documents
    pub fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        for stage in &self.stages {
            docs = stage.execute(docs)?;
        }
        Ok(docs)
    }
}

impl Stage {
    /// Parse stage from JSON
    fn from_json(stage_json: &Value) -> Result<Self> {
        if let Value::Object(obj) = stage_json {
            // Each stage should have exactly one key
            if obj.len() != 1 {
                return Err(MongoLiteError::AggregationError(
                    "Each stage must have exactly one operator".to_string(),
                ));
            }

            let (stage_name, stage_spec) = obj.iter().next().unwrap();

            match stage_name.as_str() {
                "$match" => Ok(Stage::Match(MatchStage::from_json(stage_spec)?)),
                "$project" => Ok(Stage::Project(ProjectStage::from_json(stage_spec)?)),
                "$group" => Ok(Stage::Group(GroupStage::from_json(stage_spec)?)),
                "$sort" => Ok(Stage::Sort(SortStage::from_json(stage_spec)?)),
                "$limit" => Ok(Stage::Limit(LimitStage::from_json(stage_spec)?)),
                "$skip" => Ok(Stage::Skip(SkipStage::from_json(stage_spec)?)),
                "$unwind" => Ok(Stage::Unwind(UnwindStage::from_json(stage_spec)?)),
                _ => Err(MongoLiteError::AggregationError(format!(
                    "Unknown pipeline stage: {}",
                    stage_name
                ))),
            }
        } else {
            Err(MongoLiteError::AggregationError(
                "Stage must be an object".to_string(),
            ))
        }
    }

    /// Execute this stage
    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        match self {
            Stage::Match(stage) => stage.execute(docs),
            Stage::Project(stage) => stage.execute(docs),
            Stage::Group(stage) => stage.execute(docs),
            Stage::Sort(stage) => stage.execute(docs),
            Stage::Limit(stage) => stage.execute(docs),
            Stage::Skip(stage) => stage.execute(docs),
            Stage::Unwind(stage) => stage.execute(docs),
        }
    }
}

impl MatchStage {
    fn from_json(spec: &Value) -> Result<Self> {
        let query = Query::from_json(spec)?;
        Ok(MatchStage { query })
    }

    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            // Add _id if not present (for aggregation intermediate results)
            let doc_with_id = if doc.get("_id").is_none() {
                let mut doc_obj = doc.clone();
                if let Value::Object(ref mut map) = doc_obj {
                    map.insert("_id".to_string(), Value::from(0)); // Temporary _id
                }
                doc_obj
            } else {
                doc.clone()
            };

            let doc_json_str = serde_json::to_string(&doc_with_id)?;
            let document = Document::from_json(&doc_json_str)?;

            if self.query.matches(&document) {
                results.push(doc);
            }
        }

        Ok(results)
    }
}

impl ProjectStage {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let mut fields = HashMap::new();

            for (field, value) in obj {
                let project_field = if let Some(n) = value.as_i64() {
                    match n {
                        1 => ProjectField::Include,
                        0 => ProjectField::Exclude,
                        _ => {
                            return Err(MongoLiteError::AggregationError(format!(
                                "Invalid project value: {}",
                                n
                            )))
                        }
                    }
                } else if let Some(s) = value.as_str() {
                    if s.starts_with('$') {
                        ProjectField::Rename(s.to_string())
                    } else {
                        return Err(MongoLiteError::AggregationError(format!(
                            "Invalid project expression: {}",
                            s
                        )));
                    }
                } else if let Value::Object(expr_obj) = value {
                    // Parse expression objects like {"$size": "$tags"}
                    Self::parse_expression(expr_obj)?
                } else {
                    return Err(MongoLiteError::AggregationError(
                        "Project field must be 0, 1, field reference, or expression object"
                            .to_string(),
                    ));
                };

                fields.insert(field.clone(), project_field);
            }

            Ok(ProjectStage { fields })
        } else {
            Err(MongoLiteError::AggregationError(
                "$project must be an object".to_string(),
            ))
        }
    }

    /// Parse an expression object like {"$size": "$tags"} or {"$reduce": {...}}
    fn parse_expression(obj: &serde_json::Map<String, Value>) -> Result<ProjectField> {
        if obj.len() != 1 {
            return Err(MongoLiteError::AggregationError(
                "Expression object must have exactly one operator".to_string(),
            ));
        }

        let (op, arg) = obj.iter().next().unwrap();

        match op.as_str() {
            "$size" => {
                // $size expects a field reference like "$tags"
                if let Some(field_ref) = arg.as_str() {
                    if field_ref.starts_with('$') {
                        let field_name = field_ref.trim_start_matches('$').to_string();
                        Ok(ProjectField::Expression(ProjectExpression::Size(
                            field_name,
                        )))
                    } else {
                        Err(MongoLiteError::AggregationError(
                            "$size argument must be a field reference starting with $".to_string(),
                        ))
                    }
                } else {
                    Err(MongoLiteError::AggregationError(
                        "$size argument must be a string field reference".to_string(),
                    ))
                }
            }
            "$reduce" => Self::parse_reduce_expression(arg),
            _ => Err(MongoLiteError::AggregationError(format!(
                "Unknown projection expression operator: {}",
                op
            ))),
        }
    }

    /// Parse $reduce expression
    ///
    /// Format: {input: "$arrayField", initialValue: value, in: {$op: [...]}}
    fn parse_reduce_expression(spec: &Value) -> Result<ProjectField> {
        let obj = spec.as_object().ok_or_else(|| {
            MongoLiteError::AggregationError("$reduce must be an object".to_string())
        })?;

        // Parse input field
        let input = obj.get("input").and_then(|v| v.as_str()).ok_or_else(|| {
            MongoLiteError::AggregationError("$reduce requires 'input' field reference".to_string())
        })?;

        if !input.starts_with('$') {
            return Err(MongoLiteError::AggregationError(
                "$reduce input must be a field reference starting with $".to_string(),
            ));
        }

        let input_field = input.trim_start_matches('$').to_string();

        // Parse initialValue
        let initial_value = obj.get("initialValue").cloned().ok_or_else(|| {
            MongoLiteError::AggregationError("$reduce requires 'initialValue'".to_string())
        })?;

        // Parse in expression
        let in_expr = obj.get("in").ok_or_else(|| {
            MongoLiteError::AggregationError("$reduce requires 'in' expression".to_string())
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

    /// Parse the 'in' expression of $reduce
    ///
    /// Supports: {$add: [...]}, {$multiply: [...]}, {$concat: [...]}
    /// Also supports object field references: {$add: ["$$value", "$$this.field"]}
    fn parse_reduce_in_expr(expr: &Value) -> Result<ReduceInExpr> {
        let obj = expr.as_object().ok_or_else(|| {
            MongoLiteError::AggregationError(
                "$reduce 'in' must be an expression object".to_string(),
            )
        })?;

        if obj.len() != 1 {
            return Err(MongoLiteError::AggregationError(
                "$reduce 'in' must have exactly one operator".to_string(),
            ));
        }

        let (op, args) = obj.iter().next().unwrap();

        // Check for $$this.field reference
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
                // $concat can have 2 or 3 arguments
                if let Some(arr) = args.as_array() {
                    if arr.len() == 3 {
                        // {$concat: ["$$value", separator, "$$this"]} or
                        // {$concat: ["$$value", separator, "$$this.field"]}
                        if let Some(sep) = arr.get(1).and_then(|v| v.as_str()) {
                            // Check it's not a variable reference
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
            _ => Err(MongoLiteError::AggregationError(format!(
                "Unsupported $reduce operator: {}. Supported: $add, $multiply, $concat",
                op
            ))),
        }
    }

    /// Parse $$this.field reference from arguments
    ///
    /// Returns Some(field_name) if $$this.field is found, None if just $$this
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

    /// Validate that reduce arguments contain $$value and $$this (or $$this.field)
    fn validate_reduce_args(args: &Value, op_name: &str) -> Result<()> {
        let arr = args.as_array().ok_or_else(|| {
            MongoLiteError::AggregationError(format!("{} arguments must be an array", op_name))
        })?;

        let has_value = arr.iter().any(|v| v.as_str() == Some("$$value"));
        // Accept both $$this and $$this.field
        let has_this = arr.iter().any(|v| {
            v.as_str()
                .map(|s| s == "$$this" || s.starts_with("$$this."))
                .unwrap_or(false)
        });

        if !has_value || !has_this {
            return Err(MongoLiteError::AggregationError(format!(
                "{} in $reduce must use $$value and $$this (or $$this.field)",
                op_name
            )));
        }

        Ok(())
    }

    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
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
            // Check if we're in include mode or exclude mode
            // Expression is treated as an inclusion (it produces a new field)
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

            // Determine mode: if we have any inclusions, we're in include mode
            // Exception: excluding _id is allowed in include mode
            let include_mode = has_inclusions && !has_non_id_exclusions;

            if include_mode {
                // Include mode: only include specified fields
                for (field, action) in &self.fields {
                    match action {
                        ProjectField::Include => {
                            // Use get_nested_value to support dot notation in include fields
                            if let Some(value) = get_nested_value(doc, field) {
                                result.insert(field.clone(), value.clone());
                            }
                        }
                        ProjectField::Rename(source) => {
                            let source_field = source.trim_start_matches('$');
                            // Use get_nested_value to support dot notation (e.g., "$address.city")
                            if let Some(value) = get_nested_value(doc, source_field) {
                                result.insert(field.clone(), value.clone());
                            }
                        }
                        ProjectField::Expression(expr) => {
                            let value = Self::evaluate_expression(expr, doc);
                            result.insert(field.clone(), value);
                        }
                        ProjectField::Exclude => {
                            // Should not happen in include mode
                        }
                    }
                }
            } else {
                // Exclude mode: include all fields except excluded ones
                for (field, value) in obj {
                    if let Some(action) = self.fields.get(field) {
                        match action {
                            ProjectField::Exclude => {
                                // Skip this field
                            }
                            ProjectField::Include => {
                                result.insert(field.clone(), value.clone());
                            }
                            ProjectField::Rename(_) | ProjectField::Expression(_) => {
                                // Handled below
                            }
                        }
                    } else {
                        // Field not mentioned, include it in exclude mode
                        result.insert(field.clone(), value.clone());
                    }
                }

                // Handle renames and expressions in exclude mode
                for (target_field, action) in &self.fields {
                    match action {
                        ProjectField::Rename(source) => {
                            let source_field = source.trim_start_matches('$');
                            // Use get_nested_value to support dot notation (e.g., "$address.city")
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

    /// Evaluate a projection expression against a document
    fn evaluate_expression(expr: &ProjectExpression, doc: &Value) -> Value {
        match expr {
            ProjectExpression::Size(field_name) => {
                // Get the array field and return its length
                if let Some(value) = get_nested_value(doc, field_name) {
                    if let Value::Array(arr) = value {
                        Value::Number(serde_json::Number::from(arr.len()))
                    } else {
                        // Not an array - return null (MongoDB behavior)
                        Value::Null
                    }
                } else {
                    // Field doesn't exist - return null
                    Value::Null
                }
            }
            ProjectExpression::Reduce(reduce_expr) => Self::evaluate_reduce(reduce_expr, doc),
        }
    }

    /// Evaluate a $reduce expression against a document
    ///
    /// Iterates over the input array, applying the reduction operation
    /// to accumulate a result.
    fn evaluate_reduce(expr: &ReduceExpression, doc: &Value) -> Value {
        // Get the input array
        let array = match get_nested_value(doc, &expr.input) {
            Some(Value::Array(arr)) => arr.clone(),
            _ => return Value::Null, // Not an array or missing
        };

        // Start with initial value
        let mut accumulator = expr.initial_value.clone();

        // Apply reduction for each element
        for element in array {
            accumulator = match &expr.in_expr {
                ReduceInExpr::Add => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    let elem_num = Self::value_to_f64(&element);
                    Value::from(acc_num + elem_num)
                }
                ReduceInExpr::AddField(field) => {
                    let acc_num = Self::value_to_f64(&accumulator);
                    // Get nested field from object element
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

    /// Convert a JSON value to f64 for numeric operations
    fn value_to_f64(value: &Value) -> f64 {
        match value {
            Value::Number(n) => n.as_f64().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    /// Convert a JSON value to string for concatenation
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

impl GroupStage {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            // Parse _id field
            let id = if let Some(id_value) = obj.get("_id") {
                if id_value.is_null() {
                    GroupId::Null
                } else if let Some(s) = id_value.as_str() {
                    if s.starts_with('$') {
                        GroupId::Field(s.to_string())
                    } else {
                        return Err(MongoLiteError::AggregationError(
                            "Group _id field reference must start with $".to_string(),
                        ));
                    }
                } else {
                    return Err(MongoLiteError::AggregationError(
                        "Group _id must be null or field reference".to_string(),
                    ));
                }
            } else {
                return Err(MongoLiteError::AggregationError(
                    "Group stage must have _id field".to_string(),
                ));
            };

            // Parse accumulators
            let mut accumulators = HashMap::new();
            for (field, value) in obj {
                if field == "_id" {
                    continue; // Already parsed
                }

                let accumulator = Accumulator::from_json(value)?;
                accumulators.insert(field.clone(), accumulator);
            }

            Ok(GroupStage { id, accumulators })
        } else {
            Err(MongoLiteError::AggregationError(
                "$group must be an object".to_string(),
            ))
        }
    }

    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        // Step 1: Group documents by _id expression
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();

        for doc in docs {
            let group_key = self.extract_group_key(&doc)?;
            groups.entry(group_key).or_default().push(doc);
        }

        // Step 2: Compute accumulators for each group
        let mut results = Vec::new();

        for (key, group_docs) in groups {
            let mut result = serde_json::Map::new();

            // Set _id
            result.insert("_id".to_string(), self.parse_group_key(&key)?);

            // Compute each accumulator
            for (field, accumulator) in &self.accumulators {
                let value = accumulator.compute(&group_docs)?;
                result.insert(field.clone(), value);
            }

            results.push(Value::Object(result));
        }

        Ok(results)
    }

    fn extract_group_key(&self, doc: &Value) -> Result<String> {
        match &self.id {
            GroupId::Null => Ok("__all__".to_string()),
            GroupId::Field(field) => {
                let field_name = field.trim_start_matches('$');
                // Use get_nested_value to support dot notation (e.g., "$address.city")
                if let Some(value) = get_nested_value(doc, field_name) {
                    Ok(serde_json::to_string(value)?)
                } else {
                    Ok("null".to_string())
                }
            }
        }
    }

    fn parse_group_key(&self, key: &str) -> Result<Value> {
        if key == "__all__" {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(key)?)
        }
    }
}

impl Accumulator {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            if obj.len() != 1 {
                return Err(MongoLiteError::AggregationError(
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
                            Err(MongoLiteError::AggregationError(
                                "$sum field reference must start with $".to_string(),
                            ))
                        }
                    } else {
                        Err(MongoLiteError::AggregationError(
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
                _ => Err(MongoLiteError::AggregationError(format!(
                    "Unknown accumulator: {}",
                    op
                ))),
            }
        } else {
            Err(MongoLiteError::AggregationError(
                "Accumulator must be an object".to_string(),
            ))
        }
    }

    fn compute(&self, docs: &[Value]) -> Result<Value> {
        match self {
            Accumulator::Count => Ok(Value::from(docs.len() as i64)),

            Accumulator::Sum(expr) => match expr {
                SumExpression::Constant(n) => {
                    Ok(Value::from((*n).saturating_mul(docs.len() as i64)))
                }
                SumExpression::Field(field) => {
                    let mut sum_int: i64 = 0;
                    let mut sum_float: f64 = 0.0;
                    let mut has_float = false;

                    for doc in docs {
                        // Use get_nested_value to support dot notation (e.g., "$order.total")
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
                    // Use get_nested_value to support dot notation
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

            Accumulator::Min(field) => compute_extremum(docs, field, f64::min),

            Accumulator::Max(field) => compute_extremum(docs, field, f64::max),

            Accumulator::First(field) => docs
                .first()
                // Use get_nested_value to support dot notation
                .and_then(|doc| get_nested_value(doc, field).cloned())
                .ok_or_else(|| {
                    MongoLiteError::AggregationError("No documents in group".to_string())
                }),

            Accumulator::Last(field) => docs
                .last()
                // Use get_nested_value to support dot notation
                .and_then(|doc| get_nested_value(doc, field).cloned())
                .ok_or_else(|| {
                    MongoLiteError::AggregationError("No documents in group".to_string())
                }),

            Accumulator::Push(field) => {
                // Collect all values from the field into an array
                let values: Vec<Value> = docs
                    .iter()
                    .filter_map(|doc| get_nested_value(doc, field).cloned())
                    .collect();
                Ok(Value::Array(values))
            }

            Accumulator::AddToSet(field) => {
                // Collect unique values from the field into an array
                // Use canonical JSON for equality comparison to handle
                // objects with different key ordering (MongoDB compatible)
                let mut seen = HashSet::new();
                let mut values = Vec::new();

                for doc in docs {
                    if let Some(value) = get_nested_value(doc, field) {
                        // Use canonical JSON string for uniqueness check
                        // This ensures {"a":1,"b":2} == {"b":2,"a":1}
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

impl SortStage {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
            let mut fields = Vec::new();

            for (field, value) in obj {
                let direction = if let Some(n) = value.as_i64() {
                    match n {
                        1 => SortDirection::Ascending,
                        -1 => SortDirection::Descending,
                        _ => {
                            return Err(MongoLiteError::AggregationError(
                                "Sort direction must be 1 or -1".to_string(),
                            ))
                        }
                    }
                } else {
                    return Err(MongoLiteError::AggregationError(
                        "Sort direction must be 1 or -1".to_string(),
                    ));
                };

                fields.push((field.clone(), direction));
            }

            Ok(SortStage { fields })
        } else {
            Err(MongoLiteError::AggregationError(
                "$sort must be an object".to_string(),
            ))
        }
    }

    fn execute(&self, mut docs: Vec<Value>) -> Result<Vec<Value>> {
        docs.sort_by(|a, b| {
            for (field, direction) in &self.fields {
                // Use get_nested_value to support dot notation (e.g., "address.city")
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
        (Some(a), Some(b)) => {
            // String comparison
            if let (Some(s1), Some(s2)) = (a.as_str(), b.as_str()) {
                return s1.cmp(s2);
            }

            // Number comparison
            if let (Some(n1), Some(n2)) = (a.as_f64(), b.as_f64()) {
                return n1.partial_cmp(&n2).unwrap_or(std::cmp::Ordering::Equal);
            }

            // Boolean comparison
            if let (Some(b1), Some(b2)) = (a.as_bool(), b.as_bool()) {
                return b1.cmp(&b2);
            }

            std::cmp::Ordering::Equal
        }
    }
}

impl LimitStage {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Some(n) = spec.as_u64() {
            Ok(LimitStage { limit: n as usize })
        } else {
            Err(MongoLiteError::AggregationError(
                "$limit must be a positive number".to_string(),
            ))
        }
    }

    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().take(self.limit).collect())
    }
}

impl SkipStage {
    fn from_json(spec: &Value) -> Result<Self> {
        if let Some(n) = spec.as_u64() {
            Ok(SkipStage { skip: n as usize })
        } else {
            Err(MongoLiteError::AggregationError(
                "$skip must be a positive number".to_string(),
            ))
        }
    }

    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        Ok(docs.into_iter().skip(self.skip).collect())
    }
}

impl UnwindStage {
    /// Parse $unwind stage from JSON
    ///
    /// Supports two forms:
    /// - Simple: `"$fieldName"`
    /// - Extended: `{path: "$fieldName", includeArrayIndex: "idx", preserveNullAndEmptyArrays: true}`
    fn from_json(spec: &Value) -> Result<Self> {
        // Simple form: "$fieldName"
        if let Some(s) = spec.as_str() {
            if s.starts_with('$') {
                return Ok(UnwindStage {
                    path: s.trim_start_matches('$').to_string(),
                    include_array_index: None,
                    preserve_null_and_empty_arrays: false,
                });
            }
            return Err(MongoLiteError::AggregationError(
                "$unwind path must start with $".to_string(),
            ));
        }

        // Extended form: {path: "$fieldName", ...}
        if let Value::Object(obj) = spec {
            let path = obj.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
                MongoLiteError::AggregationError("$unwind requires 'path' field".to_string())
            })?;

            if !path.starts_with('$') {
                return Err(MongoLiteError::AggregationError(
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

        Err(MongoLiteError::AggregationError(
            "$unwind must be a string or object".to_string(),
        ))
    }

    /// Execute $unwind stage
    ///
    /// For each document, if the path field is an array, outputs one document
    /// per array element with the path field replaced by that element.
    fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut results = Vec::new();

        for doc in docs {
            let array_value = get_nested_value(&doc, &self.path);

            match array_value {
                Some(Value::Array(arr)) if !arr.is_empty() => {
                    // Non-empty array: output one doc per element
                    for (index, element) in arr.iter().enumerate() {
                        let mut new_doc = doc.clone();

                        // Replace array field with single element
                        set_nested_value(&mut new_doc, &self.path, element.clone());

                        // Add index field if requested
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
                    // Empty array
                    if self.preserve_null_and_empty_arrays {
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                    // else: skip document (default MongoDB behavior)
                }
                None => {
                    // Missing field
                    if self.preserve_null_and_empty_arrays {
                        // Keep document with null value
                        let mut new_doc = doc.clone();
                        set_nested_value(&mut new_doc, &self.path, Value::Null);
                        results.push(new_doc);
                    }
                    // else: skip document
                }
                Some(Value::Null) => {
                    // Null value
                    if self.preserve_null_and_empty_arrays {
                        results.push(doc);
                    }
                    // else: skip document
                }
                Some(_) => {
                    // Not an array - treat as single-element array (MongoDB behavior)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========== Pipeline tests ==========

    #[test]
    fn test_pipeline_not_array() {
        let result = Pipeline::from_json(&json!({"$match": {}}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be an array"));
    }

    #[test]
    fn test_pipeline_empty() {
        let result = Pipeline::from_json(&json!([]));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    // ========== Stage parsing tests ==========

    #[test]
    fn test_stage_not_object() {
        let result = Stage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be an object"));
    }

    #[test]
    fn test_stage_multiple_operators() {
        let result = Stage::from_json(&json!({"$match": {}, "$sort": {"a": 1}}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exactly one operator"));
    }

    #[test]
    fn test_stage_unknown_operator() {
        let result = Stage::from_json(&json!({"$unknown": {}}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown pipeline stage"));
    }

    // ========== ProjectStage tests ==========

    #[test]
    fn test_project_exclude() {
        let docs = vec![json!({"name": "Alice", "age": 25, "secret": "hidden"})];
        let stage = ProjectStage::from_json(&json!({"secret": 0})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert!(results[0].get("name").is_some());
        assert!(results[0].get("age").is_some());
        assert!(results[0].get("secret").is_none());
    }

    #[test]
    fn test_project_rename() {
        let docs = vec![json!({"name": "Alice", "age": 25})];
        let stage = ProjectStage::from_json(&json!({"userName": "$name"})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert!(results[0].get("userName").is_some());
        assert_eq!(results[0]["userName"], "Alice");
    }

    #[test]
    fn test_project_size_expression() {
        let docs = vec![
            json!({"name": "Alice", "tags": ["rust", "python", "javascript"]}),
            json!({"name": "Bob", "tags": ["go", "java"]}),
            json!({"name": "Charlie", "tags": []}),
            json!({"name": "Dave"}), // No tags field
        ];
        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "tagCount": {"$size": "$tags"}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 4);
        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[0]["tagCount"], 3);
        assert_eq!(results[1]["name"], "Bob");
        assert_eq!(results[1]["tagCount"], 2);
        assert_eq!(results[2]["name"], "Charlie");
        assert_eq!(results[2]["tagCount"], 0);
        assert_eq!(results[3]["name"], "Dave");
        assert!(results[3]["tagCount"].is_null()); // Missing field returns null
    }

    #[test]
    fn test_project_size_non_array() {
        let docs = vec![json!({"name": "Alice", "count": 42})];
        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "countSize": {"$size": "$count"}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        // Non-array field returns null
        assert!(results[0]["countSize"].is_null());
    }

    #[test]
    fn test_project_size_invalid_arg() {
        // $size requires field reference with $
        let result = ProjectStage::from_json(&json!({
            "tagCount": {"$size": "tags"}  // Missing $ prefix
        }));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a field reference starting with $"));
    }

    #[test]
    fn test_project_size_nested_field() {
        let docs = vec![json!({"user": {"skills": ["a", "b", "c"]}})];
        let stage = ProjectStage::from_json(&json!({
            "skillCount": {"$size": "$user.skills"}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["skillCount"], 3);
    }

    #[test]
    fn test_project_invalid_value() {
        let result = ProjectStage::from_json(&json!({"field": 5}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid project value"));
    }

    #[test]
    fn test_project_invalid_expression() {
        let result = ProjectStage::from_json(&json!({"field": "not_a_ref"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid project expression"));
    }

    #[test]
    fn test_project_invalid_type() {
        let result = ProjectStage::from_json(&json!({"field": [1, 2]}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be 0, 1, field reference, or expression object"));
    }

    #[test]
    fn test_project_not_object() {
        let result = ProjectStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be an object"));
    }

    // ========== GroupStage tests ==========

    #[test]
    fn test_group_null_id() {
        let docs = vec![
            json!({"value": 10}),
            json!({"value": 20}),
            json!({"value": 30}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "total": {"$sum": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]["_id"].is_null());
        assert_eq!(results[0]["total"], 60);
    }

    #[test]
    fn test_group_missing_id() {
        let result = GroupStage::from_json(&json!({"count": {"$sum": 1}}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must have _id field"));
    }

    #[test]
    fn test_group_id_not_field_ref() {
        let result = GroupStage::from_json(&json!({"_id": "notARef"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must start with $"));
    }

    #[test]
    fn test_group_id_invalid_type() {
        let result = GroupStage::from_json(&json!({"_id": 123}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be null or field reference"));
    }

    #[test]
    fn test_group_not_object() {
        let result = GroupStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be an object"));
    }

    #[test]
    fn test_group_missing_field() {
        let docs = vec![
            json!({"city": "NYC"}),
            json!({}), // missing city
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": "$city",
            "count": {"$sum": 1}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        // Should have NYC group and null group
        assert_eq!(results.len(), 2);
    }

    // ========== Accumulator tests ==========

    #[test]
    fn test_accumulator_avg() {
        let docs = vec![
            json!({"value": 10}),
            json!({"value": 20}),
            json!({"value": 30}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "avg": {"$avg": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["avg"], 20.0);
    }

    #[test]
    fn test_accumulator_avg_empty() {
        let docs = vec![json!({})]; // No value field

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "avg": {"$avg": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert!(results[0]["avg"].is_null());
    }

    #[test]
    fn test_accumulator_min() {
        let docs = vec![
            json!({"value": 30}),
            json!({"value": 10}),
            json!({"value": 20}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "min": {"$min": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["min"], 10.0);
    }

    #[test]
    fn test_accumulator_max() {
        let docs = vec![
            json!({"value": 10}),
            json!({"value": 30}),
            json!({"value": 20}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "max": {"$max": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["max"], 30.0);
    }

    #[test]
    fn test_accumulator_first_last() {
        let docs = vec![
            json!({"value": "first"}),
            json!({"value": "middle"}),
            json!({"value": "last"}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "first": {"$first": "$value"},
            "last": {"$last": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["first"], "first");
        assert_eq!(results[0]["last"], "last");
    }

    #[test]
    fn test_accumulator_sum_float() {
        let docs = vec![json!({"value": 1.5}), json!({"value": 2.5})];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "sum": {"$sum": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["sum"], 4.0);
    }

    #[test]
    fn test_accumulator_min_max_empty() {
        let docs = vec![json!({})];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "min": {"$min": "$value"},
            "max": {"$max": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert!(results[0]["min"].is_null());
        assert!(results[0]["max"].is_null());
    }

    #[test]
    fn test_accumulator_invalid_sum_ref() {
        let result = Accumulator::from_json(&json!({"$sum": "notARef"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must start with $"));
    }

    #[test]
    fn test_accumulator_invalid_sum_type() {
        let result = Accumulator::from_json(&json!({"$sum": [1, 2]}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a number or field reference"));
    }

    #[test]
    fn test_accumulator_invalid_avg_ref() {
        let result = Accumulator::from_json(&json!({"$avg": "notARef"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_avg_type() {
        let result = Accumulator::from_json(&json!({"$avg": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_min_ref() {
        let result = Accumulator::from_json(&json!({"$min": "notARef"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_min_type() {
        let result = Accumulator::from_json(&json!({"$min": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_max_ref() {
        let result = Accumulator::from_json(&json!({"$max": "notARef"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_max_type() {
        let result = Accumulator::from_json(&json!({"$max": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_first_ref() {
        let result = Accumulator::from_json(&json!({"$first": "notARef"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_first_type() {
        let result = Accumulator::from_json(&json!({"$first": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_last_ref() {
        let result = Accumulator::from_json(&json!({"$last": "notARef"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_invalid_last_type() {
        let result = Accumulator::from_json(&json!({"$last": 123}));
        assert!(result.is_err());
    }

    #[test]
    fn test_accumulator_unknown() {
        let result = Accumulator::from_json(&json!({"$unknown": "$field"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Unknown accumulator"));
    }

    #[test]
    fn test_accumulator_not_object() {
        let result = Accumulator::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be an object"));
    }

    #[test]
    fn test_accumulator_multiple_operators() {
        let result = Accumulator::from_json(&json!({"$sum": 1, "$avg": "$x"}));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exactly one operator"));
    }

    // ========== SortStage tests ==========

    #[test]
    fn test_sort_descending() {
        let docs = vec![
            json!({"name": "Alice", "age": 25}),
            json!({"name": "Bob", "age": 35}),
            json!({"name": "Charlie", "age": 30}),
        ];

        let stage = SortStage::from_json(&json!({"age": -1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["age"], 35);
        assert_eq!(results[1]["age"], 30);
        assert_eq!(results[2]["age"], 25);
    }

    #[test]
    fn test_sort_invalid_direction_value() {
        let result = SortStage::from_json(&json!({"field": 0}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be 1 or -1"));
    }

    #[test]
    fn test_sort_invalid_direction_type() {
        let result = SortStage::from_json(&json!({"field": "asc"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be 1 or -1"));
    }

    #[test]
    fn test_sort_not_object() {
        let result = SortStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be an object"));
    }

    #[test]
    fn test_sort_by_string() {
        let docs = vec![
            json!({"name": "Charlie"}),
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
        ];

        let stage = SortStage::from_json(&json!({"name": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[1]["name"], "Bob");
        assert_eq!(results[2]["name"], "Charlie");
    }

    #[test]
    fn test_sort_by_boolean() {
        let docs = vec![
            json!({"active": true}),
            json!({"active": false}),
            json!({"active": true}),
        ];

        let stage = SortStage::from_json(&json!({"active": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["active"], false);
        assert_eq!(results[1]["active"], true);
    }

    #[test]
    fn test_sort_with_missing_field() {
        let docs = vec![
            json!({"name": "Alice", "age": 25}),
            json!({"name": "Bob"}), // missing age
            json!({"name": "Charlie", "age": 30}),
        ];

        let stage = SortStage::from_json(&json!({"age": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        // Missing value should come first
        assert_eq!(results[0]["name"], "Bob");
    }

    #[test]
    fn test_sort_multi_field() {
        let docs = vec![
            json!({"city": "NYC", "age": 30}),
            json!({"city": "LA", "age": 25}),
            json!({"city": "NYC", "age": 25}),
        ];

        let stage = SortStage::from_json(&json!({"city": 1, "age": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["city"], "LA");
        assert_eq!(results[1]["city"], "NYC");
        assert_eq!(results[1]["age"], 25);
        assert_eq!(results[2]["city"], "NYC");
        assert_eq!(results[2]["age"], 30);
    }

    // ========== LimitStage tests ==========

    #[test]
    fn test_limit_invalid() {
        let result = LimitStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a positive number"));
    }

    // ========== SkipStage tests ==========

    #[test]
    fn test_skip_invalid() {
        let result = SkipStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a positive number"));
    }

    // ========== Existing tests ==========

    #[test]
    fn test_match_stage() {
        let docs = vec![
            json!({"name": "Alice", "age": 25}),
            json!({"name": "Bob", "age": 30}),
            json!({"name": "Charlie", "age": 35}),
        ];

        let stage = MatchStage::from_json(&json!({"age": {"$gte": 30}})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["name"], "Bob");
        assert_eq!(results[1]["name"], "Charlie");
    }

    #[test]
    fn test_project_stage_include() {
        let docs = vec![json!({"name": "Alice", "age": 25, "city": "NYC"})];

        let stage = ProjectStage::from_json(&json!({"name": 1, "age": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].get("name").is_some());
        assert!(results[0].get("age").is_some());
        assert!(results[0].get("city").is_none());
    }

    #[test]
    fn test_group_stage_count() {
        let docs = vec![
            json!({"city": "NYC", "age": 25}),
            json!({"city": "LA", "age": 30}),
            json!({"city": "NYC", "age": 35}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": "$city",
            "count": {"$sum": 1}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_sort_stage() {
        let docs = vec![
            json!({"name": "Charlie", "age": 35}),
            json!({"name": "Alice", "age": 25}),
            json!({"name": "Bob", "age": 30}),
        ];

        let stage = SortStage::from_json(&json!({"age": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[1]["name"], "Bob");
        assert_eq!(results[2]["name"], "Charlie");
    }

    #[test]
    fn test_limit_stage() {
        let docs = vec![json!({"id": 1}), json!({"id": 2}), json!({"id": 3})];

        let stage = LimitStage::from_json(&json!(2)).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_skip_stage() {
        let docs = vec![json!({"id": 1}), json!({"id": 2}), json!({"id": 3})];

        let stage = SkipStage::from_json(&json!(1)).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["id"], 2);
    }

    #[test]
    fn test_full_pipeline() {
        let docs = vec![
            json!({"name": "Alice", "age": 25, "city": "NYC"}),
            json!({"name": "Bob", "age": 30, "city": "LA"}),
            json!({"name": "Charlie", "age": 35, "city": "NYC"}),
            json!({"name": "David", "age": 20, "city": "LA"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$match": {"age": {"$gte": 25}}},
            {"$group": {"_id": "$city", "count": {"$sum": 1}, "avgAge": {"$avg": "$age"}}},
            {"$sort": {"count": -1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        // NYC should be first (2 people)
        assert_eq!(results[0]["_id"], "NYC");
        assert_eq!(results[0]["count"], 2);
    }

    // ========== Dot notation tests ==========

    #[test]
    fn test_get_nested_value() {
        let doc = json!({
            "name": "Alice",
            "address": {
                "city": "NYC",
                "zip": {
                    "code": "10001"
                }
            }
        });

        assert_eq!(get_nested_value(&doc, "name"), Some(&json!("Alice")));
        assert_eq!(get_nested_value(&doc, "address.city"), Some(&json!("NYC")));
        assert_eq!(
            get_nested_value(&doc, "address.zip.code"),
            Some(&json!("10001"))
        );
        assert_eq!(get_nested_value(&doc, "nonexistent"), None);
        assert_eq!(get_nested_value(&doc, "address.nonexistent"), None);
    }

    #[test]
    fn test_group_by_nested_field() {
        let docs = vec![
            json!({"name": "Alice", "address": {"city": "NYC"}, "value": 10}),
            json!({"name": "Bob", "address": {"city": "LA"}, "value": 20}),
            json!({"name": "Charlie", "address": {"city": "NYC"}, "value": 30}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": "$address.city",
            "total": {"$sum": "$value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 2);

        // Find NYC group
        let nyc_group = results.iter().find(|r| r["_id"] == "NYC").unwrap();
        assert_eq!(nyc_group["total"], 40); // 10 + 30
    }

    #[test]
    fn test_accumulator_nested_field() {
        let docs = vec![
            json!({"order": {"total": 100, "qty": 2}}),
            json!({"order": {"total": 200, "qty": 3}}),
            json!({"order": {"total": 150, "qty": 1}}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "sumTotal": {"$sum": "$order.total"},
            "avgTotal": {"$avg": "$order.total"},
            "minQty": {"$min": "$order.qty"},
            "maxQty": {"$max": "$order.qty"},
            "firstTotal": {"$first": "$order.total"},
            "lastTotal": {"$last": "$order.total"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["sumTotal"], 450);
        assert_eq!(results[0]["avgTotal"], 150.0);
        assert_eq!(results[0]["minQty"], 1.0);
        assert_eq!(results[0]["maxQty"], 3.0);
        assert_eq!(results[0]["firstTotal"], 100);
        assert_eq!(results[0]["lastTotal"], 150);
    }

    #[test]
    fn test_project_rename_nested_field() {
        let docs = vec![json!({
            "name": "Alice",
            "address": {
                "city": "NYC",
                "street": "123 Main St"
            }
        })];

        let stage = ProjectStage::from_json(&json!({
            "userName": "$name",
            "city": "$address.city",
            "street": "$address.street"
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["userName"], "Alice");
        assert_eq!(results[0]["city"], "NYC");
        assert_eq!(results[0]["street"], "123 Main St");
    }

    #[test]
    fn test_sort_by_nested_field() {
        let docs = vec![
            json!({"name": "Charlie", "address": {"zip": 30000}}),
            json!({"name": "Alice", "address": {"zip": 10000}}),
            json!({"name": "Bob", "address": {"zip": 20000}}),
        ];

        let stage = SortStage::from_json(&json!({"address.zip": 1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["name"], "Alice");
        assert_eq!(results[1]["name"], "Bob");
        assert_eq!(results[2]["name"], "Charlie");
    }

    #[test]
    fn test_sort_by_nested_field_descending() {
        let docs = vec![
            json!({"name": "Alice", "stats": {"score": 85}}),
            json!({"name": "Bob", "stats": {"score": 92}}),
            json!({"name": "Charlie", "stats": {"score": 78}}),
        ];

        let stage = SortStage::from_json(&json!({"stats.score": -1})).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["name"], "Bob");
        assert_eq!(results[1]["name"], "Alice");
        assert_eq!(results[2]["name"], "Charlie");
    }

    #[test]
    fn test_full_pipeline_with_nested_fields() {
        let docs = vec![
            json!({"name": "Alice", "location": {"city": "NYC"}, "order": {"total": 100}}),
            json!({"name": "Bob", "location": {"city": "LA"}, "order": {"total": 200}}),
            json!({"name": "Charlie", "location": {"city": "NYC"}, "order": {"total": 150}}),
            json!({"name": "David", "location": {"city": "LA"}, "order": {"total": 300}}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {
                "_id": "$location.city",
                "totalSales": {"$sum": "$order.total"},
                "avgSales": {"$avg": "$order.total"}
            }},
            {"$sort": {"totalSales": -1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 2);

        // LA should be first with 500 total (200 + 300)
        assert_eq!(results[0]["_id"], "LA");
        assert_eq!(results[0]["totalSales"], 500);
        assert_eq!(results[0]["avgSales"], 250.0);

        // NYC should be second with 250 total (100 + 150)
        assert_eq!(results[1]["_id"], "NYC");
        assert_eq!(results[1]["totalSales"], 250);
        assert_eq!(results[1]["avgSales"], 125.0);
    }

    #[test]
    fn test_deeply_nested_field() {
        let docs = vec![
            json!({"data": {"level1": {"level2": {"value": 10}}}}),
            json!({"data": {"level1": {"level2": {"value": 20}}}}),
            json!({"data": {"level1": {"level2": {"value": 30}}}}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "sum": {"$sum": "$data.level1.level2.value"}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["sum"], 60);
    }

    #[test]
    fn test_pascal_case_nested_fields() {
        // Test with PascalCase keys like C# sends
        let docs = vec![
            json!({"Name": "TechCorp", "Location": {"Country": "USA", "City": "NYC"}, "Stats": {"Employees": 100}}),
            json!({"Name": "DataSoft", "Location": {"Country": "USA", "City": "LA"}, "Stats": {"Employees": 200}}),
            json!({"Name": "CloudNet", "Location": {"Country": "Germany", "City": "Berlin"}, "Stats": {"Employees": 150}}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {
                "_id": "$Location.Country",
                "totalEmployees": {"$sum": "$Stats.Employees"},
                "count": {"$sum": 1}
            }},
            {"$sort": {"totalEmployees": -1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        // Should have 2 groups: USA and Germany
        assert_eq!(results.len(), 2, "Expected 2 groups, got {:?}", results);

        // USA should be first with 300 total (100 + 200)
        assert_eq!(results[0]["_id"], "USA");
        assert_eq!(results[0]["totalEmployees"], 300);
        assert_eq!(results[0]["count"], 2);

        // Germany should be second with 150
        assert_eq!(results[1]["_id"], "Germany");
        assert_eq!(results[1]["totalEmployees"], 150);
        assert_eq!(results[1]["count"], 1);
    }

    // ========== $unwind stage tests ==========

    #[test]
    fn test_unwind_basic() {
        let docs = vec![json!({"items": ["a", "b", "c"], "name": "doc1"})];
        let stage = UnwindStage::from_json(&json!("$items")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["items"], "a");
        assert_eq!(results[0]["name"], "doc1");
        assert_eq!(results[1]["items"], "b");
        assert_eq!(results[2]["items"], "c");
    }

    #[test]
    fn test_unwind_with_numbers() {
        let docs = vec![json!({"values": [10, 20, 30]})];
        let stage = UnwindStage::from_json(&json!("$values")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["values"], 10);
        assert_eq!(results[1]["values"], 20);
        assert_eq!(results[2]["values"], 30);
    }

    #[test]
    fn test_unwind_with_objects() {
        let docs = vec![json!({
            "orders": [
                {"id": 1, "amount": 100},
                {"id": 2, "amount": 200}
            ]
        })];
        let stage = UnwindStage::from_json(&json!("$orders")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["orders"]["id"], 1);
        assert_eq!(results[0]["orders"]["amount"], 100);
        assert_eq!(results[1]["orders"]["id"], 2);
        assert_eq!(results[1]["orders"]["amount"], 200);
    }

    #[test]
    fn test_unwind_with_index() {
        let docs = vec![json!({"arr": ["x", "y", "z"]})];
        let stage = UnwindStage::from_json(&json!({
            "path": "$arr",
            "includeArrayIndex": "idx"
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["arr"], "x");
        assert_eq!(results[0]["idx"], 0);
        assert_eq!(results[1]["arr"], "y");
        assert_eq!(results[1]["idx"], 1);
        assert_eq!(results[2]["arr"], "z");
        assert_eq!(results[2]["idx"], 2);
    }

    #[test]
    fn test_unwind_empty_array_default() {
        let docs = vec![json!({"items": [], "name": "doc1"})];
        let stage = UnwindStage::from_json(&json!("$items")).unwrap();
        let results = stage.execute(docs).unwrap();

        // Empty array - document is skipped by default
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_unwind_empty_array_preserve() {
        let docs = vec![json!({"items": [], "name": "doc1"})];
        let stage = UnwindStage::from_json(&json!({
            "path": "$items",
            "preserveNullAndEmptyArrays": true
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        // Empty array preserved with null value
        assert_eq!(results.len(), 1);
        assert!(results[0]["items"].is_null());
        assert_eq!(results[0]["name"], "doc1");
    }

    #[test]
    fn test_unwind_missing_field_default() {
        let docs = vec![json!({"name": "doc1"})];
        let stage = UnwindStage::from_json(&json!("$items")).unwrap();
        let results = stage.execute(docs).unwrap();

        // Missing field - document is skipped by default
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_unwind_missing_field_preserve() {
        let docs = vec![json!({"name": "doc1"})];
        let stage = UnwindStage::from_json(&json!({
            "path": "$items",
            "preserveNullAndEmptyArrays": true
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        // Missing field preserved with null value
        assert_eq!(results.len(), 1);
        assert!(results[0]["items"].is_null());
    }

    #[test]
    fn test_unwind_not_array() {
        // When field is not an array, treat as single-element array
        let docs = vec![json!({"value": 42, "name": "doc1"})];
        let stage = UnwindStage::from_json(&json!("$value")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["value"], 42);
    }

    #[test]
    fn test_unwind_multiple_docs() {
        let docs = vec![
            json!({"items": ["a", "b"], "id": 1}),
            json!({"items": ["c"], "id": 2}),
            json!({"items": ["d", "e", "f"], "id": 3}),
        ];
        let stage = UnwindStage::from_json(&json!("$items")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 6);
        assert_eq!(results[0]["items"], "a");
        assert_eq!(results[0]["id"], 1);
        assert_eq!(results[1]["items"], "b");
        assert_eq!(results[1]["id"], 1);
        assert_eq!(results[2]["items"], "c");
        assert_eq!(results[2]["id"], 2);
        assert_eq!(results[3]["items"], "d");
        assert_eq!(results[3]["id"], 3);
    }

    #[test]
    fn test_unwind_nested_field() {
        let docs = vec![json!({"data": {"tags": ["rust", "mongodb"]}})];
        let stage = UnwindStage::from_json(&json!("$data.tags")).unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["data"]["tags"], "rust");
        assert_eq!(results[1]["data"]["tags"], "mongodb");
    }

    #[test]
    fn test_unwind_pipeline_integration() {
        // Test $unwind in a full pipeline with $match and $group
        let docs = vec![
            json!({"category": "A", "items": [1, 2, 3]}),
            json!({"category": "B", "items": [10, 20]}),
            json!({"category": "A", "items": [4, 5]}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$unwind": "$items"},
            {"$group": {
                "_id": "$category",
                "total": {"$sum": "$items"}
            }},
            {"$sort": {"_id": 1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["_id"], "A");
        assert_eq!(results[0]["total"], 15); // 1+2+3+4+5
        assert_eq!(results[1]["_id"], "B");
        assert_eq!(results[1]["total"], 30); // 10+20
    }

    #[test]
    fn test_unwind_parse_error_no_dollar() {
        let result = UnwindStage::from_json(&json!("items"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must start with $"));
    }

    #[test]
    fn test_unwind_parse_error_missing_path() {
        let result = UnwindStage::from_json(&json!({"includeArrayIndex": "idx"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("requires 'path'"));
    }

    // ========== $reduce expression tests ==========

    #[test]
    fn test_reduce_sum() {
        let docs = vec![json!({"numbers": [1, 2, 3, 4, 5]})];
        let stage = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["total"], 15.0);
    }

    #[test]
    fn test_reduce_sum_floats() {
        let docs = vec![json!({"values": [1.5, 2.5, 3.0]})];
        let stage = ProjectStage::from_json(&json!({
            "sum": {"$reduce": {
                "input": "$values",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["sum"], 7.0);
    }

    #[test]
    fn test_reduce_multiply() {
        let docs = vec![json!({"numbers": [2, 3, 4]})];
        let stage = ProjectStage::from_json(&json!({
            "product": {"$reduce": {
                "input": "$numbers",
                "initialValue": 1,
                "in": {"$multiply": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["product"], 24.0);
    }

    #[test]
    fn test_reduce_concat() {
        let docs = vec![json!({"words": ["Hello", " ", "World"]})];
        let stage = ProjectStage::from_json(&json!({
            "message": {"$reduce": {
                "input": "$words",
                "initialValue": "",
                "in": {"$concat": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["message"], "Hello World");
    }

    #[test]
    fn test_reduce_concat_with_separator() {
        let docs = vec![json!({"tags": ["rust", "mongodb", "db"]})];
        let stage = ProjectStage::from_json(&json!({
            "tagList": {"$reduce": {
                "input": "$tags",
                "initialValue": "",
                "in": {"$concat": ["$$value", ", ", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["tagList"], "rust, mongodb, db");
    }

    #[test]
    fn test_reduce_empty_array() {
        let docs = vec![json!({"numbers": []})];
        let stage = ProjectStage::from_json(&json!({
            "sum": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        // Empty array returns initial value
        assert_eq!(results[0]["sum"], 0);
    }

    #[test]
    fn test_reduce_missing_field() {
        let docs = vec![json!({"name": "test"})];
        let stage = ProjectStage::from_json(&json!({
            "sum": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        // Missing field returns null
        assert!(results[0]["sum"].is_null());
    }

    #[test]
    fn test_reduce_nested_field() {
        let docs = vec![json!({"data": {"scores": [10, 20, 30]}})];
        let stage = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$data.scores",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["total"], 60.0);
    }

    #[test]
    fn test_reduce_with_other_projections() {
        let docs = vec![json!({"name": "Test", "values": [1, 2, 3]})];
        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "sum": {"$reduce": {
                "input": "$values",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }))
        .unwrap();
        let results = stage.execute(docs).unwrap();

        assert_eq!(results[0]["name"], "Test");
        assert_eq!(results[0]["sum"], 6.0);
    }

    #[test]
    fn test_reduce_in_pipeline() {
        let docs = vec![
            json!({"category": "A", "prices": [10, 20, 30]}),
            json!({"category": "B", "prices": [5, 15]}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$project": {
                "category": 1,
                "total": {"$reduce": {
                    "input": "$prices",
                    "initialValue": 0,
                    "in": {"$add": ["$$value", "$$this"]}
                }}
            }},
            {"$sort": {"total": -1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        assert_eq!(results[0]["category"], "A");
        assert_eq!(results[0]["total"], 60.0);
        assert_eq!(results[1]["category"], "B");
        assert_eq!(results[1]["total"], 20.0);
    }

    #[test]
    fn test_reduce_parse_error_missing_input() {
        let result = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("input"));
    }

    #[test]
    fn test_reduce_parse_error_missing_initial_value() {
        let result = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$numbers",
                "in": {"$add": ["$$value", "$$this"]}
            }}
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("initialValue"));
    }

    #[test]
    fn test_reduce_parse_error_missing_in() {
        let result = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0
            }}
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("in"));
    }

    #[test]
    fn test_reduce_parse_error_unsupported_operator() {
        let result = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0,
                "in": {"$subtract": ["$$value", "$$this"]}
            }}
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));
    }

    #[test]
    fn test_reduce_parse_error_missing_variables() {
        let result = ProjectStage::from_json(&json!({
            "total": {"$reduce": {
                "input": "$numbers",
                "initialValue": 0,
                "in": {"$add": [1, 2]}  // Missing $$value and $$this
            }}
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("$$value"));
    }

    // ========== $push ACCUMULATOR TESTS ==========

    #[test]
    fn test_push_basic() {
        let docs = vec![
            json!({"category": "A", "item": "apple"}),
            json!({"category": "A", "item": "banana"}),
            json!({"category": "B", "item": "cherry"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$category", "items": {"$push": "$item"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        // Find category A result
        let cat_a = results.iter().find(|r| r["_id"] == "A").unwrap();
        let items_a = cat_a["items"].as_array().unwrap();
        assert_eq!(items_a.len(), 2);
        assert!(items_a.contains(&json!("apple")));
        assert!(items_a.contains(&json!("banana")));

        // Find category B result
        let cat_b = results.iter().find(|r| r["_id"] == "B").unwrap();
        let items_b = cat_b["items"].as_array().unwrap();
        assert_eq!(items_b.len(), 1);
        assert!(items_b.contains(&json!("cherry")));
    }

    #[test]
    fn test_push_with_numbers() {
        let docs = vec![
            json!({"group": "X", "value": 10}),
            json!({"group": "X", "value": 20}),
            json!({"group": "X", "value": 30}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$group", "values": {"$push": "$value"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let values = results[0]["values"].as_array().unwrap();
        assert_eq!(values.len(), 3);
        assert!(values.contains(&json!(10)));
        assert!(values.contains(&json!(20)));
        assert!(values.contains(&json!(30)));
    }

    #[test]
    fn test_push_null_group() {
        let docs = vec![
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
            json!({"name": "Charlie"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "allNames": {"$push": "$name"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        let names = results[0]["allNames"].as_array().unwrap();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn test_push_with_dot_notation() {
        let docs = vec![
            json!({"user": {"name": "Alice"}, "type": "admin"}),
            json!({"user": {"name": "Bob"}, "type": "admin"}),
            json!({"user": {"name": "Charlie"}, "type": "user"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$type", "names": {"$push": "$user.name"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        let admin = results.iter().find(|r| r["_id"] == "admin").unwrap();
        let admin_names = admin["names"].as_array().unwrap();
        assert_eq!(admin_names.len(), 2);
        assert!(admin_names.contains(&json!("Alice")));
        assert!(admin_names.contains(&json!("Bob")));
    }

    // ========== $addToSet ACCUMULATOR TESTS ==========

    #[test]
    fn test_addtoset_basic() {
        let docs = vec![
            json!({"category": "A", "tag": "red"}),
            json!({"category": "A", "tag": "blue"}),
            json!({"category": "A", "tag": "red"}), // duplicate
            json!({"category": "B", "tag": "green"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$category", "uniqueTags": {"$addToSet": "$tag"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        let cat_a = results.iter().find(|r| r["_id"] == "A").unwrap();
        let tags_a = cat_a["uniqueTags"].as_array().unwrap();
        assert_eq!(tags_a.len(), 2); // Only unique: red, blue
        assert!(tags_a.contains(&json!("red")));
        assert!(tags_a.contains(&json!("blue")));

        let cat_b = results.iter().find(|r| r["_id"] == "B").unwrap();
        let tags_b = cat_b["uniqueTags"].as_array().unwrap();
        assert_eq!(tags_b.len(), 1);
    }

    #[test]
    fn test_addtoset_all_duplicates() {
        let docs = vec![
            json!({"group": "X", "status": "active"}),
            json!({"group": "X", "status": "active"}),
            json!({"group": "X", "status": "active"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$group", "statuses": {"$addToSet": "$status"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let statuses = results[0]["statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), 1); // All duplicates collapsed
        assert_eq!(statuses[0], "active");
    }

    #[test]
    fn test_addtoset_with_numbers() {
        let docs = vec![
            json!({"type": "score", "value": 100}),
            json!({"type": "score", "value": 200}),
            json!({"type": "score", "value": 100}), // duplicate
            json!({"type": "score", "value": 300}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$type", "uniqueScores": {"$addToSet": "$value"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let scores = results[0]["uniqueScores"].as_array().unwrap();
        assert_eq!(scores.len(), 3); // 100, 200, 300
    }

    #[test]
    fn test_addtoset_null_group() {
        let docs = vec![
            json!({"color": "red"}),
            json!({"color": "blue"}),
            json!({"color": "red"}),
            json!({"color": "green"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "allColors": {"$addToSet": "$color"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let colors = results[0]["allColors"].as_array().unwrap();
        assert_eq!(colors.len(), 3); // red, blue, green
    }

    #[test]
    fn test_addtoset_object_key_order_independence() {
        // This test verifies that $addToSet correctly deduplicates objects
        // that have the same fields but different key insertion order.
        // MongoDB treats {"a":1,"b":2} and {"b":2,"a":1} as equal.
        let docs = vec![
            json!({"category": "A", "data": {"x": 1, "y": 2}}),
            json!({"category": "A", "data": {"y": 2, "x": 1}}), // Same as above, different key order
            json!({"category": "A", "data": {"x": 1, "y": 3}}), // Different value
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": "$category", "uniqueData": {"$addToSet": "$data"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let unique_data = results[0]["uniqueData"].as_array().unwrap();

        // Should have only 2 unique values, not 3
        // (the first two documents have equivalent data objects)
        assert_eq!(
            unique_data.len(),
            2,
            "Expected 2 unique values but got {}: {:?}",
            unique_data.len(),
            unique_data
        );
    }

    #[test]
    fn test_addtoset_nested_object_key_order() {
        // Test with nested objects having different key orders
        let docs = vec![
            json!({"id": 1, "nested": {"outer": {"a": 1, "b": 2}}}),
            json!({"id": 2, "nested": {"outer": {"b": 2, "a": 1}}}), // Same, different order
            json!({"id": 3, "nested": {"outer": {"a": 1, "c": 3}}}), // Different
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {"_id": null, "uniqueNested": {"$addToSet": "$nested"}}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        let unique_nested = results[0]["uniqueNested"].as_array().unwrap();

        // Should have only 2 unique nested objects
        assert_eq!(
            unique_nested.len(),
            2,
            "Expected 2 unique nested objects but got {}: {:?}",
            unique_nested.len(),
            unique_nested
        );
    }

    // ========== $reduce WITH OBJECT ARRAYS TESTS ==========

    #[test]
    fn test_reduce_object_array_sum() {
        // Sum prices from array of objects
        let docs = vec![json!({
            "name": "Order1",
            "items": [
                {"name": "apple", "price": 10},
                {"name": "banana", "price": 20},
                {"name": "cherry", "price": 30}
            ]
        })];

        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "totalPrice": {"$reduce": {
                "input": "$items",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this.price"]}
            }}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["name"], "Order1");
        assert_eq!(results[0]["totalPrice"], 60.0);
    }

    #[test]
    fn test_reduce_object_array_multiply() {
        let docs = vec![json!({
            "name": "Test",
            "factors": [
                {"factor": 2},
                {"factor": 3},
                {"factor": 4}
            ]
        })];

        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "product": {"$reduce": {
                "input": "$factors",
                "initialValue": 1,
                "in": {"$multiply": ["$$value", "$$this.factor"]}
            }}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["product"], 24.0); // 2 * 3 * 4
    }

    #[test]
    fn test_reduce_object_array_concat() {
        let docs = vec![json!({
            "id": 1,
            "people": [
                {"name": "Alice"},
                {"name": "Bob"},
                {"name": "Charlie"}
            ]
        })];

        let stage = ProjectStage::from_json(&json!({
            "id": 1,
            "allNames": {"$reduce": {
                "input": "$people",
                "initialValue": "",
                "in": {"$concat": ["$$value", "$$this.name"]}
            }}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["allNames"], "AliceBobCharlie");
    }

    #[test]
    fn test_reduce_object_array_concat_with_separator() {
        let docs = vec![json!({
            "id": 1,
            "tags": [
                {"label": "rust"},
                {"label": "mongodb"},
                {"label": "database"}
            ]
        })];

        let stage = ProjectStage::from_json(&json!({
            "id": 1,
            "tagString": {"$reduce": {
                "input": "$tags",
                "initialValue": "",
                "in": {"$concat": ["$$value", ", ", "$$this.label"]}
            }}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["tagString"], "rust, mongodb, database");
    }

    #[test]
    fn test_reduce_object_array_nested_field() {
        // Access nested fields within objects
        let docs = vec![json!({
            "name": "Store",
            "products": [
                {"details": {"price": 100}},
                {"details": {"price": 200}},
                {"details": {"price": 50}}
            ]
        })];

        let stage = ProjectStage::from_json(&json!({
            "name": 1,
            "total": {"$reduce": {
                "input": "$products",
                "initialValue": 0,
                "in": {"$add": ["$$value", "$$this.details.price"]}
            }}
        }))
        .unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["total"], 350.0);
    }

    #[test]
    fn test_reduce_object_array_in_pipeline() {
        let docs = vec![
            json!({
                "orderId": "A",
                "items": [{"price": 10}, {"price": 20}]
            }),
            json!({
                "orderId": "B",
                "items": [{"price": 5}, {"price": 15}, {"price": 25}]
            }),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$project": {
                "orderId": 1,
                "orderTotal": {"$reduce": {
                    "input": "$items",
                    "initialValue": 0,
                    "in": {"$add": ["$$value", "$$this.price"]}
                }}
            }},
            {"$sort": {"orderTotal": -1}}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();

        assert_eq!(results[0]["orderId"], "B");
        assert_eq!(results[0]["orderTotal"], 45.0);
        assert_eq!(results[1]["orderId"], "A");
        assert_eq!(results[1]["orderTotal"], 30.0);
    }

    #[test]
    fn test_push_and_addtoset_combined() {
        let docs = vec![
            json!({"dept": "Sales", "name": "Alice", "skill": "Excel"}),
            json!({"dept": "Sales", "name": "Bob", "skill": "Excel"}),
            json!({"dept": "Sales", "name": "Charlie", "skill": "Python"}),
        ];

        let pipeline = Pipeline::from_json(&json!([
            {"$group": {
                "_id": "$dept",
                "allNames": {"$push": "$name"},
                "uniqueSkills": {"$addToSet": "$skill"}
            }}
        ]))
        .unwrap();

        let results = pipeline.execute(docs).unwrap();
        assert_eq!(results.len(), 1);

        let names = results[0]["allNames"].as_array().unwrap();
        assert_eq!(names.len(), 3); // All names

        let skills = results[0]["uniqueSkills"].as_array().unwrap();
        assert_eq!(skills.len(), 2); // Only unique: Excel, Python
    }
}
