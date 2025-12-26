// src/aggregation/stages/group_stage.rs
// $group stage implementation
//
// STREAMING OPTIMIZATION (2024-12):
// Changed from storing full documents per group to streaming accumulator states.
// Memory usage: O(N * doc_size) → O(G * state_size) where G = number of groups
// This fixes OOM issues with large collections (e.g., 650K emails → 15 groups)

use crate::aggregation::types::{Accumulator, AccumulatorState, GroupId, GroupStage};
use crate::error::{IronBaseError, Result};
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
                        return Err(IronBaseError::AggregationError(
                            "Group _id field reference must start with $".to_string(),
                        ));
                    }
                } else {
                    return Err(IronBaseError::AggregationError(
                        "Group _id must be null or field reference".to_string(),
                    ));
                }
            } else {
                return Err(IronBaseError::AggregationError(
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
            Err(IronBaseError::AggregationError(
                "$group must be an object".to_string(),
            ))
        }
    }

    /// Execute $group stage with streaming accumulators (memory optimized)
    ///
    /// OLD APPROACH (BAD - OOM prone):
    /// ```ignore
    /// groups: HashMap<String, Vec<Value>>  // Stored ALL documents per group
    /// ```
    ///
    /// NEW APPROACH (GOOD - constant memory per group):
    /// ```ignore
    /// groups: HashMap<String, HashMap<String, AccumulatorState>>  // Only accumulated state
    /// ```
    ///
    /// Memory comparison for 650K emails grouped by sender (~10K unique senders):
    /// - OLD: 650K × ~800 bytes = ~500MB
    /// - NEW: 10K × ~64 bytes = ~640KB (780× reduction!)
    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        // HashMap<group_key, HashMap<accumulator_field_name, AccumulatorState>>
        let mut groups: HashMap<String, HashMap<String, AccumulatorState>> = HashMap::new();

        for doc in docs {
            let group_key = self.extract_group_key(&doc)?;

            // Get or initialize accumulator states for this group
            let group_states = groups.entry(group_key).or_insert_with(|| {
                self.accumulators
                    .iter()
                    .map(|(field, acc)| (field.clone(), acc.init_state()))
                    .collect()
            });

            // Update each accumulator state with this document (streaming)
            for (field, accumulator) in &self.accumulators {
                if let Some(state) = group_states.get_mut(field) {
                    state.update(&doc, accumulator);
                }
            }
            // doc is DROPPED here - not stored in memory!
        }

        // Finalize: convert accumulated states to output values
        let mut results = Vec::new();

        for (key, mut group_states) in groups {
            let mut result = serde_json::Map::new();

            result.insert("_id".to_string(), self.parse_group_key(&key)?);

            for field in self.accumulators.keys() {
                if let Some(state) = group_states.remove(field) {
                    let value = state.finalize();
                    result.insert(field.clone(), value);
                }
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

    /// Execute $group stage with streaming document input (iterator-based)
    ///
    /// This is the most memory-efficient approach for large collections:
    /// - Documents are processed one at a time from the iterator
    /// - Only accumulator states are kept in memory (not full documents)
    /// - Memory usage: O(G × state_size) where G = number of groups
    ///
    /// # Example memory savings for 650K emails grouped by sender (10K groups):
    /// - Vec<Value> input: 650K × 800 bytes = ~500MB loaded upfront
    /// - Iterator input: ~0MB (documents processed and discarded)
    /// - Group states: 10K × 64 bytes = ~640KB
    /// - **Total: 640KB instead of 500MB+**
    pub(crate) fn execute_streaming<I>(&self, docs: I) -> Result<Vec<Value>>
    where
        I: Iterator<Item = Result<Value>>,
    {
        let mut groups: HashMap<String, HashMap<String, AccumulatorState>> = HashMap::new();

        for doc_result in docs {
            let doc = doc_result?;
            let group_key = self.extract_group_key(&doc)?;

            let group_states = groups.entry(group_key).or_insert_with(|| {
                self.accumulators
                    .iter()
                    .map(|(field, acc)| (field.clone(), acc.init_state()))
                    .collect()
            });

            for (field, accumulator) in &self.accumulators {
                if let Some(state) = group_states.get_mut(field) {
                    state.update(&doc, accumulator);
                }
            }
            // doc is dropped here - NOT kept in memory!
        }

        // Finalize
        let mut results = Vec::new();
        for (key, mut group_states) in groups {
            let mut result = serde_json::Map::new();
            result.insert("_id".to_string(), self.parse_group_key(&key)?);

            for field in self.accumulators.keys() {
                if let Some(state) = group_states.remove(field) {
                    let value = state.finalize();
                    result.insert(field.clone(), value);
                }
            }
            results.push(Value::Object(result));
        }

        Ok(results)
    }
}
