//! Index and search tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use ironbase_core::find_options::{apply_projection, apply_sort};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use super::helpers::{validate_collection_name, DEFAULT_QUERY_LIMIT};
use super::params::{
    ExplainParams, FindWithHintParams, FulltextIndexParams, FulltextSearchParams, FuzzyIndexParams,
    FuzzySearchParams, IndexCreateParams, IndexDropParams, IndexListParams, ParseParams,
};

/// Dispatch index tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "index_create" => handle_index_create(params, adapter),
        "index_list" => handle_index_list(params, adapter),
        "index_create_fuzzy" => handle_index_create_fuzzy(params, adapter),
        "index_create_fulltext" => handle_index_create_fulltext(params, adapter),
        "index_list_fulltext" => handle_index_list_fulltext(params, adapter),
        "index_drop" => handle_index_drop(params, adapter),
        "fuzzy_search" => handle_fuzzy_search(params, adapter),
        "fulltext_search" => handle_fulltext_search(params, adapter),
        "explain" => handle_explain(params, adapter),
        "find_with_hint" => handle_find_with_hint(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown index tool: {}",
            name
        ))),
    }
}

/// Parse projection Value to HashMap<String, i32>
fn parse_projection_value(proj: Option<Value>) -> Result<Option<HashMap<String, i32>>> {
    match proj {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) => {
            let mut result = HashMap::new();
            for (key, value) in map {
                let v = value.as_i64().unwrap_or(1) as i32;
                result.insert(key, v);
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(_) => Err(McpError::invalid_params(
            "Projection must be an object like {\"field\": 1} or {\"field\": 0}",
        )),
    }
}

/// Parse sort specification from Value to Vec<(String, i32)>
fn parse_sort_value(sort: Option<Value>) -> Result<Option<Vec<(String, i32)>>> {
    match sort {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
            let mut result = Vec::new();
            for item in arr {
                let pair = item.as_array().ok_or_else(|| {
                    McpError::invalid_params("Sort array items must be [field, direction] pairs")
                })?;
                if pair.len() != 2 {
                    return Err(McpError::invalid_params(
                        "Sort array items must have exactly 2 elements",
                    ));
                }
                let field = pair[0]
                    .as_str()
                    .ok_or_else(|| McpError::invalid_params("Sort field must be a string"))?;
                let direction = pair[1]
                    .as_i64()
                    .ok_or_else(|| McpError::invalid_params("Sort direction must be 1 or -1"))?;
                result.push((field.to_string(), direction as i32));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(Value::Object(map)) => {
            let mut result = Vec::new();
            for (key, value) in map {
                let direction = value.as_i64().unwrap_or(1) as i32;
                result.push((key, direction));
            }
            if result.is_empty() {
                Ok(None)
            } else {
                Ok(Some(result))
            }
        }
        Some(_) => Err(McpError::invalid_params("Sort must be an array or object")),
    }
}

fn handle_index_create(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexCreateParams = IndexCreateParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Check for compound index (fields takes priority)
    if let Some(fields) = p.fields {
        if fields.is_empty() {
            return Err(McpError::invalid_params("fields array is empty"));
        }
        let name = adapter.create_compound_index(&p.collection, &fields, p.unique, p.sparse)?;
        Ok(json!({
            "index_name": name,
            "fields": fields,
            "unique": p.unique,
            "sparse": p.sparse
        }))
    } else if let Some(field) = p.field {
        // Single field index
        let name = adapter.create_index(&p.collection, &field, p.unique, p.sparse)?;
        Ok(json!({
            "index_name": name,
            "field": field,
            "unique": p.unique,
            "sparse": p.sparse
        }))
    } else {
        Err(McpError::invalid_params(
            "Either 'field' or 'fields' must be provided",
        ))
    }
}

fn handle_index_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let indexes = adapter.list_indexes(&p.collection)?;
    Ok(json!({"indexes": indexes}))
}

fn handle_index_create_fuzzy(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FuzzyIndexParams = FuzzyIndexParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let name = adapter.create_fuzzy_index(&p.collection, &p.field, &p.algorithm, p.threshold)?;
    Ok(json!({
        "index_name": name,
        "field": p.field,
        "algorithm": p.algorithm,
        "threshold": p.threshold
    }))
}

fn handle_index_create_fulltext(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FulltextIndexParams = FulltextIndexParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let name = adapter.create_fulltext_index(
        &p.collection,
        &p.field,
        &p.language,
        p.min_word_length,
        p.accent_folding,
    )?;
    Ok(json!({
        "index_name": name,
        "field": p.field,
        "language": p.language,
        "min_word_length": p.min_word_length.unwrap_or(2),
        "accent_folding": p.accent_folding.unwrap_or(true)
    }))
}

fn handle_index_list_fulltext(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let indexes = adapter.list_fulltext_indexes(&p.collection)?;
    Ok(json!({"indexes": indexes, "count": indexes.len()}))
}

fn handle_index_drop(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexDropParams = IndexDropParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    adapter.drop_index(&p.collection, &p.index_name)?;
    Ok(json!({"success": true, "dropped": p.index_name}))
}

fn handle_fuzzy_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FuzzySearchParams = FuzzySearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let threshold = p.threshold;
    let algorithm = p.algorithm.as_deref();
    let limit = p
        .limit
        .unwrap_or(DEFAULT_QUERY_LIMIT)
        .min(DEFAULT_QUERY_LIMIT);
    let projection = parse_projection_value(p.projection)?;

    // Use the real fuzzy search with index
    let mut results =
        adapter.fuzzy_search(&p.collection, &p.field, &p.query, threshold, algorithm)?;

    // Apply limit
    results.truncate(limit);

    // Format results with scores, applying projection if specified
    let documents: Vec<Value> = results
        .into_iter()
        .map(|(doc, score)| {
            let projected_doc = if let Some(ref proj) = projection {
                apply_projection(&doc, proj).map_err(|e| McpError::invalid_params(e.to_string()))
            } else {
                Ok(doc)
            }?;
            Ok(json!({
                "document": projected_doc,
                "score": score
            }))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(json!({"results": documents, "count": documents.len()}))
}

fn handle_fulltext_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FulltextSearchParams = FulltextSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let projection = parse_projection_value(p.projection)?;

    let options = FulltextSearchOptions {
        limit: p.limit,
        skip: p.skip,
        min_score: p.min_score,
        projection,
    };
    let results = adapter.fulltext_search(&p.collection, &p.field, &p.query, options)?;

    // Format results with scores and matched tokens
    let documents: Vec<Value> = results
        .into_iter()
        .map(|(doc, score, matched_tokens)| {
            json!({
                "document": doc,
                "score": score,
                "matched_tokens": matched_tokens
            })
        })
        .collect();

    Ok(json!({"results": documents, "count": documents.len()}))
}

fn handle_explain(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ExplainParams = ExplainParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let plan = adapter.explain(&p.collection, p.query)?;
    Ok(json!({"plan": plan}))
}

fn handle_find_with_hint(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FindWithHintParams = FindWithHintParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let projection = parse_projection_value(p.projection)?;
    let sort = parse_sort_value(p.sort)?;

    let mut documents = adapter.find_with_hint(&p.collection, p.query, &p.hint)?;

    // Apply sort if specified
    if let Some(ref sort_spec) = sort {
        apply_sort(&mut documents, sort_spec)
            .map_err(|e| McpError::invalid_params(e.to_string()))?;
    }

    // Apply skip
    if let Some(s) = p.skip {
        if s < documents.len() {
            documents = documents.into_iter().skip(s).collect();
        } else {
            documents = Vec::new();
        }
    }

    // Apply limit
    if let Some(l) = p.limit {
        documents.truncate(l.min(DEFAULT_QUERY_LIMIT));
    }

    // Apply projection if specified
    let documents: Vec<Value> = if let Some(ref proj) = projection {
        documents
            .into_iter()
            .map(|doc| apply_projection(&doc, proj))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| McpError::invalid_params(e.to_string()))?
    } else {
        documents
    };

    Ok(json!({"documents": documents, "count": documents.len()}))
}
