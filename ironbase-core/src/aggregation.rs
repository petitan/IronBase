// src/aggregation.rs
// Aggregation pipeline implementation

use crate::document::Document;
use crate::error::{MongoLiteError, Result};
use crate::query::Query;
use crate::value_utils::get_nested_value;
use serde_json::Value;
use std::collections::HashMap;

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
    Include,        // 1
    Exclude,        // 0
    Rename(String), // "$fieldName"
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
                } else {
                    return Err(MongoLiteError::AggregationError(
                        "Project field must be 0, 1, or field reference".to_string(),
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
            let has_inclusions = self
                .fields
                .values()
                .any(|f| matches!(f, ProjectField::Include | ProjectField::Rename(_)));
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
                            ProjectField::Rename(_) => {
                                // Handled below
                            }
                        }
                    } else {
                        // Field not mentioned, include it in exclude mode
                        result.insert(field.clone(), value.clone());
                    }
                }

                // Handle renames in exclude mode
                for (target_field, action) in &self.fields {
                    if let ProjectField::Rename(source) = action {
                        let source_field = source.trim_start_matches('$');
                        // Use get_nested_value to support dot notation (e.g., "$address.city")
                        if let Some(value) = get_nested_value(doc, source_field) {
                            result.insert(target_field.clone(), value.clone());
                        }
                    }
                }
            }
        }

        Ok(Value::Object(result))
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
            groups.entry(group_key).or_insert_with(Vec::new).push(doc);
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
                SumExpression::Constant(n) => Ok(Value::from((*n).saturating_mul(docs.len() as i64))),
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
                let mut count = 0;

                for doc in docs {
                    // Use get_nested_value to support dot notation
                    if let Some(value) = get_nested_value(doc, field) {
                        if let Some(n) = value.as_f64() {
                            sum += n;
                            count += 1;
                        } else if let Some(n) = value.as_i64() {
                            sum += n as f64;
                            count += 1;
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
        assert!(result.unwrap_err().to_string().contains("must be an object"));
    }

    #[test]
    fn test_stage_multiple_operators() {
        let result = Stage::from_json(&json!({"$match": {}, "$sort": {"a": 1}}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exactly one operator"));
    }

    #[test]
    fn test_stage_unknown_operator() {
        let result = Stage::from_json(&json!({"$unknown": {}}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown pipeline stage"));
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
    fn test_project_invalid_value() {
        let result = ProjectStage::from_json(&json!({"field": 5}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid project value"));
    }

    #[test]
    fn test_project_invalid_expression() {
        let result = ProjectStage::from_json(&json!({"field": "not_a_ref"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid project expression"));
    }

    #[test]
    fn test_project_invalid_type() {
        let result = ProjectStage::from_json(&json!({"field": [1, 2]}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be 0, 1, or field reference"));
    }

    #[test]
    fn test_project_not_object() {
        let result = ProjectStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be an object"));
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
        })).unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0]["_id"].is_null());
        assert_eq!(results[0]["total"], 60);
    }

    #[test]
    fn test_group_missing_id() {
        let result = GroupStage::from_json(&json!({"count": {"$sum": 1}}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must have _id field"));
    }

    #[test]
    fn test_group_id_not_field_ref() {
        let result = GroupStage::from_json(&json!({"_id": "notARef"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must start with $"));
    }

    #[test]
    fn test_group_id_invalid_type() {
        let result = GroupStage::from_json(&json!({"_id": 123}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be null or field reference"));
    }

    #[test]
    fn test_group_not_object() {
        let result = GroupStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be an object"));
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
        })).unwrap();

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
        })).unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["avg"], 20.0);
    }

    #[test]
    fn test_accumulator_avg_empty() {
        let docs = vec![json!({})]; // No value field

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "avg": {"$avg": "$value"}
        })).unwrap();

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
        })).unwrap();

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
        })).unwrap();

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
        })).unwrap();

        let results = stage.execute(docs).unwrap();
        assert_eq!(results[0]["first"], "first");
        assert_eq!(results[0]["last"], "last");
    }

    #[test]
    fn test_accumulator_sum_float() {
        let docs = vec![
            json!({"value": 1.5}),
            json!({"value": 2.5}),
        ];

        let stage = GroupStage::from_json(&json!({
            "_id": null,
            "sum": {"$sum": "$value"}
        })).unwrap();

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
        })).unwrap();

        let results = stage.execute(docs).unwrap();
        assert!(results[0]["min"].is_null());
        assert!(results[0]["max"].is_null());
    }

    #[test]
    fn test_accumulator_invalid_sum_ref() {
        let result = Accumulator::from_json(&json!({"$sum": "notARef"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must start with $"));
    }

    #[test]
    fn test_accumulator_invalid_sum_type() {
        let result = Accumulator::from_json(&json!({"$sum": [1, 2]}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be a number or field reference"));
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
        assert!(result.unwrap_err().to_string().contains("Unknown accumulator"));
    }

    #[test]
    fn test_accumulator_not_object() {
        let result = Accumulator::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be an object"));
    }

    #[test]
    fn test_accumulator_multiple_operators() {
        let result = Accumulator::from_json(&json!({"$sum": 1, "$avg": "$x"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exactly one operator"));
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
        assert!(result.unwrap_err().to_string().contains("must be an object"));
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
        assert!(result.unwrap_err().to_string().contains("must be a positive number"));
    }

    // ========== SkipStage tests ==========

    #[test]
    fn test_skip_invalid() {
        let result = SkipStage::from_json(&json!("invalid"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("must be a positive number"));
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
        assert_eq!(get_nested_value(&doc, "address.zip.code"), Some(&json!("10001")));
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
        })).unwrap();

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
        })).unwrap();

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
        })).unwrap();

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
        ])).unwrap();

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
        })).unwrap();

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
        ])).unwrap();

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
}
