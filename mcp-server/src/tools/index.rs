//! Index and search tool handlers
//!
//! Uses typed parameter structs for compile-time validation.

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use ironbase_mcp_types::responses::{
    BTreeIndexCreated, CompoundIndexCreated, ExplainResponse, FulltextAnalyzeResponse, FulltextHit,
    FulltextIndexCreated, FulltextSearchResponse, FuzzyHit, FuzzyIndexCreated, FuzzySearchResponse,
    IndexCreateResponse, IndexDropResponse, IndexListAll, IndexListResponse,
    IndexStatsRefreshResponse, IndexStatsResponse, MatchScope, TokenAnalysis, TypedIndexList,
};

use super::fusion::apply_document_qualification;
use super::helpers::{
    document_fields, parse_projection_value, parse_sort_value, validate_collection_name,
    DEFAULT_QUERY_LIMIT,
};
use super::params::{
    ExplainParams, FindWithHintParams, FulltextAnalyzeParams, FulltextIndexParams,
    FulltextSearchParams, FuzzyIndexParams, FuzzySearchParams, IndexCreateArgs, IndexCreateParams,
    IndexDropParams, IndexListParams, ParseParams,
};

/// Dispatch index tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "index_create" => handle_index_create(params, adapter),
        "index_list" => handle_index_list(params, adapter),
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

/// Create an index of any type, routed by the `type` discriminator
/// (default `btree`; see [`IndexCreateArgs`]): each type parses into its own
/// typed struct, which ignores the unrelated fields.
fn handle_index_create(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let response = match IndexCreateArgs::parse(params)? {
        IndexCreateArgs::BTree(p) => handle_index_create_btree(p, adapter)?,
        IndexCreateArgs::Fulltext(p) => handle_index_create_fulltext(p, adapter)?,
        IndexCreateArgs::Fuzzy(p) => handle_index_create_fuzzy(p, adapter)?,
        IndexCreateArgs::Vector(p) => super::vector::handle_index_create_vector(p, adapter)?,
    };
    Ok(serde_json::to_value(response)?)
}

fn handle_index_create_btree(
    p: IndexCreateParams,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<IndexCreateResponse> {
    validate_collection_name(&p.collection)?;

    // Check for compound index (fields takes priority)
    if let Some(fields) = p.fields {
        if fields.is_empty() {
            return Err(McpError::invalid_params("fields array is empty"));
        }
        let index_name =
            adapter.create_compound_index(&p.collection, &fields, p.unique, p.sparse)?;
        Ok(IndexCreateResponse::Compound(CompoundIndexCreated {
            index_name,
            fields,
            unique: p.unique,
            sparse: p.sparse,
        }))
    } else if let Some(field) = p.field {
        // Single field index
        let index_name = adapter.create_index(&p.collection, &field, p.unique, p.sparse)?;
        Ok(IndexCreateResponse::BTree(BTreeIndexCreated {
            index_name,
            field,
            unique: p.unique,
            sparse: p.sparse,
        }))
    } else {
        Err(McpError::invalid_params(
            "Either 'field' or 'fields' must be provided",
        ))
    }
}

/// List indexes. With no `type`, returns every subtype (B+ tree, fulltext, fuzzy,
/// vector). With a `type`, returns only that subtype's list under `indexes`.
fn handle_index_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let response = match p.index_type.as_deref() {
        Some("btree") => {
            IndexListResponse::BTree(TypedIndexList::new(adapter.list_indexes(&p.collection)?))
        }
        Some("fulltext") => IndexListResponse::Fulltext(TypedIndexList::new(
            adapter.fulltext_index_infos(&p.collection)?,
        )),
        Some("fuzzy") => IndexListResponse::Fuzzy(TypedIndexList::new(
            adapter.fuzzy_index_infos(&p.collection)?,
        )),
        Some("vector") => IndexListResponse::Vector(TypedIndexList::new(
            adapter.vector_index_infos(&p.collection)?,
        )),
        Some(other) => {
            return Err(McpError::invalid_params(format!(
                "Unknown index type '{}'. Use one of: btree, fulltext, fuzzy, vector",
                other
            )))
        }
        None => {
            // No filter: list all subtypes (fuzzy now included — was missing before)
            let btree_indexes = adapter.list_indexes(&p.collection)?;
            let fulltext_indexes = adapter
                .fulltext_index_infos(&p.collection)
                .unwrap_or_default();
            let fuzzy_indexes = adapter.fuzzy_index_infos(&p.collection).unwrap_or_default();
            let vector_indexes = adapter
                .vector_index_infos(&p.collection)
                .unwrap_or_default();

            IndexListResponse::All(IndexListAll::new(
                btree_indexes,
                fulltext_indexes,
                fuzzy_indexes,
                vector_indexes,
            ))
        }
    };
    Ok(serde_json::to_value(response)?)
}

fn handle_index_create_fuzzy(
    p: FuzzyIndexParams,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<IndexCreateResponse> {
    validate_collection_name(&p.collection)?;

    let index_name =
        adapter.create_fuzzy_index(&p.collection, &p.field, &p.algorithm, p.threshold)?;
    Ok(IndexCreateResponse::Fuzzy(FuzzyIndexCreated {
        index_name,
        field: p.field,
        algorithm: p.algorithm,
        threshold: p.threshold,
    }))
}

fn handle_index_create_fulltext(
    p: FulltextIndexParams,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<IndexCreateResponse> {
    validate_collection_name(&p.collection)?;

    let index_name = adapter.create_fulltext_index(
        &p.collection,
        &p.field,
        &p.language,
        p.min_word_length,
        p.accent_folding,
    )?;
    Ok(IndexCreateResponse::Fulltext(FulltextIndexCreated {
        index_name,
        field: p.field,
        language: p.language,
        min_word_length: p.min_word_length.unwrap_or(2),
        accent_folding: p.accent_folding.unwrap_or(true),
    }))
}

/// Drop an index of any subtype. Vector indexes are routed to the dedicated
/// `drop_vector_index` (which also removes the on-disk HNSW cache file and the
/// `vector_indexes` metadata that the generic `drop_index` does not touch);
/// btree/fulltext/fuzzy go through the generic path.
fn handle_index_drop(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexDropParams = IndexDropParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let is_vector = adapter
        .list_vector_indexes(&p.collection)
        .map(|v| {
            v.iter()
                .any(|i| i.get("name").and_then(|n| n.as_str()) == Some(&p.index_name))
        })
        .unwrap_or(false);

    if is_vector {
        adapter.drop_vector_index(&p.collection, &p.index_name)?;
    } else {
        adapter.drop_index(&p.collection, &p.index_name)?;
    }
    Ok(serde_json::to_value(IndexDropResponse {
        success: true,
        dropped: p.index_name,
    })?)
}

/// Refresh statistics for all B+ tree indexes in a collection.
///
/// The query planner uses these statistics to estimate selectivity
/// and choose the best index for a query.
fn handle_index_stats_refresh(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    adapter.refresh_index_stats(&p.collection)?;
    Ok(serde_json::to_value(IndexStatsRefreshResponse {
        success: true,
        message: format!(
            "Index statistics refreshed for collection '{}'",
            p.collection
        ),
    })?)
}

/// Get detailed statistics for all B+ tree indexes in a collection.
///
/// Returns num_keys, distinct_count, has_histogram, has_mcv for each index.
fn handle_index_stats(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: IndexListParams = IndexListParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let response = match p.index_type.as_deref().unwrap_or("btree") {
        "btree" => IndexStatsResponse::BTree(TypedIndexList::new(
            adapter.get_index_statistics(&p.collection)?,
        )),
        "fulltext" => IndexStatsResponse::Fulltext(TypedIndexList::new(
            adapter.fulltext_index_infos(&p.collection)?,
        )),
        "fuzzy" => IndexStatsResponse::Fuzzy(TypedIndexList::new(
            adapter.fuzzy_index_infos(&p.collection)?,
        )),
        "vector" => IndexStatsResponse::Vector(TypedIndexList::new(
            adapter.vector_index_infos(&p.collection)?,
        )),
        other => {
            return Err(McpError::invalid_params(format!(
                "Unknown index type '{}'. Use one of: btree, fulltext, fuzzy, vector",
                other
            )))
        }
    };
    Ok(serde_json::to_value(response)?)
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

    // Flat shape, consistent with fulltext_search and hybrid_search (#68 follow-up,
    // v1.0.501): document fields at the top level + `_`-prefixed engine metadata.
    let results = results
        .into_iter()
        .map(|r| {
            Ok(FuzzyHit {
                document: document_fields(r.document)?,
                score: r.score,
                matched_value: r.matched_value,
                highlight: r.highlight,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(serde_json::to_value(FuzzySearchResponse {
        count: results.len(),
        results,
    })?)
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
        // Bounded like fuzzy_search: the limit sizes candidate and result sets
        limit: p.limit.map(|l| l.min(DEFAULT_QUERY_LIMIT)),
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

    // Flat shape, consistent with hybrid_search (#68, v1.0.501): document fields
    // at the top level, engine metadata with `_`-prefix to avoid colliding with
    // a user field named e.g. "score". Clients write one parser for both tools.
    let results = results
        .into_iter()
        .map(|res| {
            Ok(FulltextHit {
                document: document_fields(res.document)?,
                score: res.score,
                matched_tokens: res.matched_tokens,
                highlights: res.highlights,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // #4: Always include match_scope in response (consistent with hybrid_search)
    let match_scope = if qual.is_doc_scope {
        MatchScope::Document
    } else {
        MatchScope::Chunk
    };
    Ok(serde_json::to_value(FulltextSearchResponse {
        count: results.len(),
        results,
        match_scope,
        qualified_doc_ids: qual.qualified_doc_count,
    })?)
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
    Ok(serde_json::to_value(FulltextAnalyzeResponse {
        input: result.input,
        language: result.language,
        accent_folding: result.accent_folding,
        min_word_length: result.min_word_length,
        tokens: result
            .tokens
            .into_iter()
            .map(|t| TokenAnalysis {
                original: t.original,
                normalized: t.normalized,
                stemmed: t.stemmed,
                kept: t.kept,
                filtered_reason: t.filtered_reason,
            })
            .collect(),
        stop_words_removed: result.stop_words_removed,
        final_tokens: result.final_tokens,
        inherited_from_index: inherited,
    })?)
}

fn handle_explain(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: ExplainParams = ExplainParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let plan = adapter.explain(&p.collection, p.query)?;
    Ok(serde_json::to_value(ExplainResponse { plan })?)
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
