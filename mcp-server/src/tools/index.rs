//! Index and search tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::fusion::apply_document_qualification;
use super::helpers::{
    parse_projection_value, parse_sort_value, validate_collection_name, DEFAULT_QUERY_LIMIT,
};
use super::params::{
    ExplainParams, FindWithHintParams, FulltextAnalyzeParams, FulltextIndexParams,
    FulltextSearchParams, FuzzyIndexParams, FuzzySearchParams, IndexCreateParams, IndexDropParams,
    IndexListParams, ParseParams,
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
        "index_stats_refresh" => handle_index_stats_refresh(params, adapter),
        "index_stats" => handle_index_stats(params, adapter),
        "fuzzy_search" => handle_fuzzy_search(params, adapter),
        "fulltext_search" => handle_fulltext_search(params, adapter),
        "fulltext_analyze" => handle_fulltext_analyze(params, adapter),
        "explain" => handle_explain(params, adapter),
        "find_with_hint" => handle_find_with_hint(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown index tool: {}",
            name
        ))),
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

    // Get all index types
    let btree_indexes = adapter.list_indexes(&p.collection)?;
    let fulltext_indexes = adapter
        .list_fulltext_indexes(&p.collection)
        .unwrap_or_default();
    let vector_indexes = adapter
        .list_vector_indexes(&p.collection)
        .unwrap_or_default();

    Ok(json!({
        "btree_indexes": btree_indexes,
        "fulltext_indexes": fulltext_indexes,
        "vector_indexes": vector_indexes,
        "summary": {
            "btree_count": btree_indexes.len(),
            "fulltext_count": fulltext_indexes.len(),
            "vector_count": vector_indexes.len(),
            "total": btree_indexes.len() + fulltext_indexes.len() + vector_indexes.len()
        }
    }))
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

/// Refresh statistics for all B+ tree indexes in a collection.
///
/// The query planner uses these statistics to estimate selectivity
/// and choose the best index for a query.
fn handle_index_stats_refresh(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    adapter.refresh_index_stats(&p.collection)?;
    Ok(json!({
        "success": true,
        "message": format!("Index statistics refreshed for collection '{}'", p.collection)
    }))
}

/// Get detailed statistics for all B+ tree indexes in a collection.
///
/// Returns num_keys, distinct_count, has_histogram, has_mcv for each index.
fn handle_index_stats(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let stats = adapter.get_index_statistics(&p.collection)?;
    Ok(json!({
        "indexes": stats,
        "count": stats.len()
    }))
}

fn handle_fuzzy_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    use crate::adapter::FuzzySearchOptions;

    let p: FuzzySearchParams = FuzzySearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Parse algorithm string to enum
    let algorithm = p.algorithm.as_deref().and_then(|algo| {
        use ironbase_core::FuzzyAlgorithm;
        match algo {
            "levenshtein" => Some(FuzzyAlgorithm::Levenshtein),
            "damerau_levenshtein" => Some(FuzzyAlgorithm::DamerauLevenshtein),
            "jaro_winkler" => Some(FuzzyAlgorithm::JaroWinkler),
            _ => None,
        }
    });

    let projection = parse_projection_value(p.projection)?;

    // Build FuzzySearchOptions - all processing happens in core
    let options = FuzzySearchOptions {
        algorithm,
        threshold: p.threshold,
        limit: Some(
            p.limit
                .unwrap_or(DEFAULT_QUERY_LIMIT)
                .min(DEFAULT_QUERY_LIMIT),
        ),
        skip: p.skip,
        projection,
        filter: p.filter,
        highlight: p.highlight,
    };

    // Core handles everything: filter, projection, highlight
    let results = adapter.fuzzy_search_ext(&p.collection, &p.field, &p.query, options)?;

    // Format results
    let documents: Vec<Value> = results
        .into_iter()
        .map(|r| {
            let mut result = json!({
                "document": r.document,
                "score": r.score,
                "matched_value": r.matched_value
            });
            // Add highlight if present
            if let Some(highlight) = r.highlight {
                result["highlight"] = json!(highlight);
            }
            result
        })
        .collect();

    Ok(json!({"results": documents, "count": documents.len()}))
}

fn handle_fulltext_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: FulltextSearchParams = FulltextSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    if p.query.is_empty() {
        return Err(McpError::invalid_params("Query cannot be empty"));
    }

    let projection = parse_projection_value(p.projection)?;

    // Collect qualification fields: all fields to qualify across (multi-field union)
    let qual_fields: Vec<&str> = if let Some(ref fields) = p.fields {
        fields.iter().map(|s| s.as_str()).collect()
    } else {
        vec![&p.field]
    };

    // Shared qualification: match_scope validation, multi-field union, filter merge
    let qual = apply_document_qualification(
        adapter,
        &p.collection,
        &qual_fields,
        &p.query,
        p.mode.as_deref(),
        p.match_scope.as_deref(),
        p.filter,
    )?;

    let options = FulltextSearchOptions {
        limit: p.limit,
        skip: p.skip,
        min_score: p.min_score,
        projection,
        filter: qual.effective_filter,
        and_mode: qual.effective_and_mode,
        highlight: p.highlight,
        highlight_context: p.highlight_context,
        highlight_max_snippets: p.highlight_max_snippets,
        target_doc_ids: None,
    };
    // Multi-field search if `fields` is provided, otherwise single-field
    let results = if let Some(ref fields) = p.fields {
        let field_refs: Vec<&str> = fields.iter().map(|s| s.as_str()).collect();
        adapter.fulltext_search_multi(&p.collection, &field_refs, &p.query, options)?
    } else {
        adapter.fulltext_search(&p.collection, &p.field, &p.query, options)?
    };

    // Format results with scores, matched tokens, and optional highlights
    let documents: Vec<Value> = results
        .into_iter()
        .map(|res| {
            let mut result = json!({
                "document": res.document,
                "score": res.score,
                "matched_tokens": res.matched_tokens
            });
            // Add highlights if present
            if let Some(highlights) = res.highlights {
                result["highlights"] = json!(highlights);
            }
            result
        })
        .collect();

    // #4: Always include match_scope in response (consistent with hybrid_search)
    let match_scope = if qual.is_doc_scope {
        "document"
    } else {
        "chunk"
    };
    let mut response = json!({
        "results": documents,
        "count": documents.len(),
        "match_scope": match_scope
    });
    if let Some(count) = qual.qualified_doc_count {
        response["qualified_doc_ids"] = json!(count);
    }
    Ok(response)
}

/// Handle fulltext_analyze - debug tokenization process
fn handle_fulltext_analyze(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    use ironbase_core::fulltext::{analyze_text, FtsLanguage, FtsOptions};

    let p: FulltextAnalyzeParams = FulltextAnalyzeParams::parse(params)?;

    // If a collection+field is given, inherit that index's real tokenization config
    // so the debugger reflects exactly how that index processes text (#65). Otherwise
    // build options from the explicit params (pure tokenizer test).
    let (options, inherited) = match (p.collection.as_deref(), p.field.as_deref()) {
        (Some(collection), Some(field)) => {
            let opts = adapter.get_fulltext_index_options(collection, field)?;
            (opts, true)
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(McpError::invalid_params(
                "fulltext_analyze: 'collection' and 'field' must be provided together",
            ));
        }
        (None, None) => {
            let language = match p.language.to_lowercase().as_str() {
                "hungarian" | "hu" => FtsLanguage::Hungarian,
                "english" | "en" => FtsLanguage::English,
                "german" | "de" => FtsLanguage::German,
                _ => FtsLanguage::None,
            };
            (
                FtsOptions {
                    language,
                    min_word_length: p.min_word_length.unwrap_or(2),
                    accent_folding: p.accent_folding,
                },
                false,
            )
        }
    };

    let result = analyze_text(&p.text, &options);
    let mut out = json!(result);
    if let Value::Object(ref mut obj) = out {
        obj.insert("inherited_from_index".to_string(), json!(inherited));
    }
    Ok(out)
}

fn handle_explain(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ExplainParams = ExplainParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let plan = adapter.explain(&p.collection, p.query)?;
    Ok(json!({"plan": plan}))
}

fn handle_find_with_hint(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    use ironbase_core::find_options::FindOptions;

    let p: FindWithHintParams = FindWithHintParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Build FindOptions - all processing happens in core
    let mut options = FindOptions::with_safe_defaults();

    // Set projection if specified
    if let Some(proj) = parse_projection_value(p.projection)? {
        options = options.with_projection(proj);
    }

    // Set sort if specified
    if let Some(sort_spec) = parse_sort_value(p.sort)? {
        options = options.with_sort(sort_spec);
    }

    // Set skip if specified
    if let Some(s) = p.skip {
        options = options.with_skip(s);
    }

    // Set limit (bounded by DEFAULT_QUERY_LIMIT for safety)
    let limit = p
        .limit
        .map(|l| l.min(DEFAULT_QUERY_LIMIT))
        .unwrap_or(DEFAULT_QUERY_LIMIT);
    options = options.with_limit(limit);

    // Core handles everything: sort, skip, limit, projection, OOM protection
    let documents = adapter.find_with_hint_ext(&p.collection, p.query, &p.hint, options)?;

    Ok(json!({"documents": documents, "count": documents.len()}))
}
