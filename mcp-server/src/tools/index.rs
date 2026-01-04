//! Index and search tool handlers

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use ironbase_core::find_options::{apply_projection, apply_sort};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::{
    get_string, parse_limit, parse_projection, parse_skip, parse_sort, parse_threshold,
    validate_collection_name, MAX_QUERY_LIMIT,
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
        _ => Err(McpError::InvalidParams(format!(
            "Unknown index tool: {}",
            name
        ))),
    }
}

fn handle_index_create(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let unique = params
        .get("unique")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let sparse = params
        .get("sparse")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Check for compound index
    if let Some(fields) = params.get("fields").and_then(|v| v.as_array()) {
        let field_names: Vec<String> = fields
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if field_names.is_empty() {
            return Err(McpError::InvalidParams("fields array is empty".to_string()));
        }
        let name = adapter.create_compound_index(&collection, &field_names, unique, sparse)?;
        Ok(json!({"index_name": name, "fields": field_names, "unique": unique, "sparse": sparse}))
    } else {
        // Single field index
        let field = get_string(&params, "field")?;
        let name = adapter.create_index(&collection, &field, unique, sparse)?;
        Ok(json!({"index_name": name, "field": field, "unique": unique, "sparse": sparse}))
    }
}

fn handle_index_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let indexes = adapter.list_indexes(&collection)?;
    Ok(json!({"indexes": indexes}))
}

fn handle_index_create_fuzzy(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let field = get_string(&params, "field")?;
    let algorithm = params
        .get("algorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("jaro_winkler");
    let threshold = params
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.8);
    let name = adapter.create_fuzzy_index(&collection, &field, algorithm, threshold)?;
    Ok(json!({
        "index_name": name,
        "field": field,
        "algorithm": algorithm,
        "threshold": threshold
    }))
}

fn handle_index_create_fulltext(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let field = get_string(&params, "field")?;
    let language = params
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    let min_word_length = params
        .get("min_word_length")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let accent_folding = params.get("accent_folding").and_then(|v| v.as_bool());
    let name = adapter.create_fulltext_index(
        &collection,
        &field,
        language,
        min_word_length,
        accent_folding,
    )?;
    Ok(json!({
        "index_name": name,
        "field": field,
        "language": language,
        "min_word_length": min_word_length.unwrap_or(2),
        "accent_folding": accent_folding.unwrap_or(true)
    }))
}

fn handle_index_list_fulltext(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let indexes = adapter.list_fulltext_indexes(&collection)?;
    Ok(json!({"indexes": indexes, "count": indexes.len()}))
}

fn handle_index_drop(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let index_name = get_string(&params, "index_name")?;
    adapter.drop_index(&collection, &index_name)?;
    Ok(json!({"success": true, "dropped": index_name}))
}

fn handle_fuzzy_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let field = get_string(&params, "field")?;
    let query = get_string(&params, "query")?;
    let threshold = parse_threshold(&params)?;
    let algorithm = params.get("algorithm").and_then(|v| v.as_str());
    let limit = parse_limit(&params).or(Some(MAX_QUERY_LIMIT));
    let projection = parse_projection(&params)?;

    // Use the real fuzzy search with index
    let mut results = adapter.fuzzy_search(&collection, &field, &query, threshold, algorithm)?;

    // Apply limit if specified (capped at MAX_QUERY_LIMIT)
    if let Some(lim) = limit {
        results.truncate(lim);
    }

    // Format results with scores, applying projection if specified
    let documents: Vec<Value> = results
        .into_iter()
        .map(|(doc, score)| {
            let projected_doc = if let Some(ref proj) = projection {
                apply_projection(&doc, proj).map_err(|e| McpError::InvalidParams(e.to_string()))
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
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let field = get_string(&params, "field")?;
    let query = get_string(&params, "query")?;
    let limit = parse_limit(&params);
    let skip = parse_skip(&params);
    let min_score = params.get("min_score").and_then(|v| v.as_f64());

    // Parse projection with validation
    let projection = parse_projection(&params)?;

    let options = FulltextSearchOptions {
        limit,
        skip,
        min_score,
        projection,
    };
    let results = adapter.fulltext_search(&collection, &field, &query, options)?;

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
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let plan = adapter.explain(&collection, query)?;
    Ok(json!({"plan": plan}))
}

fn handle_find_with_hint(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collection = get_string(&params, "collection")?;
    validate_collection_name(&collection)?;
    let query = params.get("query").cloned().unwrap_or(json!({}));
    let hint = get_string(&params, "hint")?;
    let projection = parse_projection(&params)?;
    let sort = parse_sort(&params);
    let limit = parse_limit(&params);
    let skip = parse_skip(&params);

    let mut documents = adapter.find_with_hint(&collection, query, &hint)?;

    // Apply sort if specified
    if let Some(ref sort_spec) = sort {
        apply_sort(&mut documents, sort_spec)
            .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    }

    // Apply skip
    if let Some(s) = skip {
        if s < documents.len() {
            documents = documents.into_iter().skip(s).collect();
        } else {
            documents = Vec::new();
        }
    }

    // Apply limit
    if let Some(l) = limit {
        documents.truncate(l.min(MAX_QUERY_LIMIT));
    }

    // Apply projection if specified
    let documents: Vec<Value> = if let Some(ref proj) = projection {
        documents
            .into_iter()
            .map(|doc| apply_projection(&doc, proj))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| McpError::InvalidParams(e.to_string()))?
    } else {
        documents
    };

    Ok(json!({"documents": documents, "count": documents.len()}))
}
