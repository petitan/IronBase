// src/aggregation/stages/group_stage.rs
// $group stage implementation

use crate::aggregation::types::{Accumulator, GroupId, GroupStage};
use crate::error::{MongoLiteError, Result};
use crate::value_utils::get_nested_value;
use serde_json::Value;
use std::collections::HashMap;

impl GroupStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        if let Value::Object(obj) = spec {
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

            let mut accumulators = HashMap::new();
            for (field, value) in obj {
                if field == "_id" {
                    continue;
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

    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        let mut groups: HashMap<String, Vec<Value>> = HashMap::new();

        for doc in docs {
            let group_key = self.extract_group_key(&doc)?;
            groups.entry(group_key).or_default().push(doc);
        }

        let mut results = Vec::new();

        for (key, group_docs) in groups {
            let mut result = serde_json::Map::new();

            result.insert("_id".to_string(), self.parse_group_key(&key)?);

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
