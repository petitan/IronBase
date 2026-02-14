//! RAG (Retrieval-Augmented Generation) tool handlers
//!
//! Provides simplified MCP tools for semantic search:
//! - rag_collection_create: Set up collection with indexes
//! - rag_document_import: Chunk + embed + insert
//! - rag_search: Auto-embed query + hybrid search (THE KEY TOOL!)
//! - rag_collection_stats: RAG-specific statistics
//!
//! Key difference from hybrid_search: rag_search auto-embeds the query!

use crate::adapter::{FulltextSearchOptions, IronBaseAdapter};
use crate::chunking::{chunk_content, ChunkMode, ChunkOptions};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_EMBEDDING_PROVIDER, DEFAULT_TEXT_FIELD};
use super::helpers::{parse_projection_value, validate_collection_name};
use super::params::{
    ParseParams, RagCollectionCreateParams, RagCollectionStatsParams, RagDocumentImportParams,
    RagSearchParams,
};

/// System collection for RAG configs
const RAG_CONFIG_COLLECTION: &str = "_system.rag";

/// RRF constant - empirically optimal value (Cormack et al., 2009)
const RRF_K: f64 = 60.0;

/// Maximum internal limit to prevent OOM
const MAX_INTERNAL_LIMIT: usize = 1000;

/// Reserved metadata keys that cannot be overwritten by user input (security)
const RESERVED_METADATA_KEYS: &[&str] = &[
    "_id",
    "doc_id",
    "chunk_index",
    "chunk_total",
    "start_char",
    "end_char",
];

// ============================================================================
// RAG Config Storage
// ============================================================================

/// RAG configuration stored in _system.rag collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub collection: String,
    pub embedding_field: String,
    pub text_field: String,
    pub provider: String,
    pub language: String,
    pub dimension: usize,
    pub created_at: String,
}

/// Get RAG config for a collection from _system.rag
fn get_rag_config(adapter: &IronBaseAdapter, collection: &str) -> Result<Option<RagConfig>> {
    // Try to find config
    let result = adapter.find_one(RAG_CONFIG_COLLECTION, json!({"collection": collection}));

    match result {
        Ok(Some(doc)) => {
            let config: RagConfig = serde_json::from_value(doc)
                .map_err(|e| McpError::internal(format!("Invalid RAG config: {}", e)))?;
            Ok(Some(config))
        }
        Ok(None) => Ok(None),
        Err(_) => {
            // Collection might not exist yet - that's OK
            Ok(None)
        }
    }
}

/// Save RAG config to _system.rag collection
fn save_rag_config(adapter: &IronBaseAdapter, config: &RagConfig) -> Result<()> {
    // Ensure system collection exists (log errors but continue)
    if let Err(e) = adapter.create_collection(RAG_CONFIG_COLLECTION) {
        tracing::debug!("RAG config collection creation: {} (may already exist)", e);
    }

    // Upsert config
    let filter = json!({"collection": config.collection});
    let config_json =
        serde_json::to_value(config).map_err(|e| McpError::internal(e.to_string()))?;

    // Try update first, if no match then insert
    let update_result = adapter.update_one(
        RAG_CONFIG_COLLECTION,
        filter.clone(),
        json!({"$set": config_json}),
    );

    match update_result {
        Ok(result) if result.modified_count > 0 => Ok(()),
        _ => {
            // Insert new config
            let config_json =
                serde_json::to_value(config).map_err(|e| McpError::internal(e.to_string()))?;
            adapter.insert_one(RAG_CONFIG_COLLECTION, config_json)?;
            Ok(())
        }
    }
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch RAG tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "rag_collection_create" => handle_rag_collection_create(params, adapter, embedding_manager),
        "rag_document_import" => handle_rag_document_import(params, adapter, embedding_manager),
        "rag_search" => handle_rag_search(params, adapter, embedding_manager),
        "rag_collection_stats" => handle_rag_collection_stats(params, adapter, embedding_manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown RAG tool: {}",
            name
        ))),
    }
}

// ============================================================================
// rag_collection_create Handler
// ============================================================================

fn handle_rag_collection_create(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: RagCollectionCreateParams = RagCollectionCreateParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Get embedding manager
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
        )
    })?;

    // Validate provider exists
    let provider = manager.get_provider(&p.provider).ok_or_else(|| {
        let available: Vec<_> = manager
            .list_models()
            .iter()
            .map(|m| m.provider.clone())
            .collect();
        McpError::invalid_params(format!(
            "Provider '{}' not found. Available: {:?}",
            p.provider, available
        ))
    })?;

    let dimension = provider.dimension();

    // 1. Create collection if not exists
    let collection_created = match adapter.create_collection(&p.collection) {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!("Collection creation: {} (may already exist)", e);
            false
        }
    };

    // 2. Create vector index (HNSW)
    let vector_index_created = match adapter.create_vector_index(
        &p.collection,
        &p.embedding_field,
        dimension,
        "cosine",
        100_000, // max_vectors
        16,      // m
        100,     // ef_construction
        50,      // ef_search
    ) {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!("Vector index creation: {} (may already exist)", e);
            false
        }
    };

    // 3. Create fulltext index
    let fulltext_index_created = match adapter.create_fulltext_index(
        &p.collection,
        &p.text_field,
        &p.language,
        Some(2),    // min_word_length
        Some(true), // accent_folding
    ) {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!("Fulltext index creation: {} (may already exist)", e);
            false
        }
    };

    // 4. Save RAG config
    let config = RagConfig {
        collection: p.collection.clone(),
        embedding_field: p.embedding_field.clone(),
        text_field: p.text_field.clone(),
        provider: p.provider.clone(),
        language: p.language.clone(),
        dimension,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    save_rag_config(adapter, &config)?;

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "config": {
            "embedding_field": p.embedding_field,
            "text_field": p.text_field,
            "provider": p.provider,
            "language": p.language,
            "dimension": dimension
        },
        "indexes": {
            "collection_created": collection_created,
            "vector_created": vector_index_created,
            "fulltext_created": fulltext_index_created
        }
    }))
}

// ============================================================================
// rag_document_import Handler
// ============================================================================

fn handle_rag_document_import(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: RagDocumentImportParams = RagDocumentImportParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    if p.content.is_empty() {
        return Err(McpError::invalid_params("Content cannot be empty"));
    }

    // Get embedding manager
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
        )
    })?;

    // Get RAG config or use defaults
    let rag_config = get_rag_config(adapter, &p.collection)?;
    let (embedding_field, text_field, provider_name) = match &rag_config {
        Some(cfg) => (
            cfg.embedding_field.clone(),
            cfg.text_field.clone(),
            p.provider.clone().unwrap_or_else(|| cfg.provider.clone()),
        ),
        None => (
            DEFAULT_EMBEDDING_FIELD.to_string(),
            DEFAULT_TEXT_FIELD.to_string(),
            p.provider
                .clone()
                .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string()),
        ),
    };

    // Get provider
    let provider = manager.get_provider(&provider_name).ok_or_else(|| {
        McpError::invalid_params(format!("Provider '{}' not available", provider_name))
    })?;

    // Chunk content
    let mode = ChunkMode::parse(&p.mode);
    let options = ChunkOptions::default()
        .with_chunk_size(p.chunk_size)
        .with_overlap(p.overlap)
        .with_mode(mode);

    let chunks = chunk_content(&p.content, &options)
        .map_err(|e| McpError::internal(format!("Chunking failed: {}", e)))?;

    if chunks.is_empty() {
        return Ok(json!({
            "success": true,
            "collection": p.collection,
            "chunks_created": 0,
            "message": "No chunks generated from content"
        }));
    }

    // Generate embeddings in batches (OOM protection)
    let mut all_embeddings: Vec<Vec<f32>> = Vec::new();
    all_embeddings.try_reserve(chunks.len()).map_err(|e| {
        McpError::internal(format!(
            "Cannot allocate memory for {} embeddings: {}",
            chunks.len(),
            e
        ))
    })?;

    for batch in chunks.chunks(100) {
        let texts: Vec<&str> = batch.iter().map(|c| c.text.as_str()).collect();
        let embeddings = provider
            .embed_batch(&texts)
            .map_err(|e| McpError::internal(format!("Embedding failed: {}", e)))?;
        all_embeddings.extend(embeddings);
    }

    // Ensure collection and indexes exist if no RAG config (idempotent)
    if rag_config.is_none() {
        let _ = adapter.create_collection(&p.collection);
        let _ = adapter.create_vector_index(
            &p.collection,
            &embedding_field,
            provider.dimension(),
            "cosine",
            100_000,
            16,
            100,
            50,
        );
        let _ =
            adapter.create_fulltext_index(&p.collection, &text_field, "none", Some(2), Some(true));
    }

    // Generate parent doc_id
    let parent_id = p.doc_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Build documents
    let mut documents: Vec<Value> = Vec::with_capacity(chunks.len());
    for (chunk, embedding) in chunks.iter().zip(all_embeddings.iter()) {
        let mut doc = json!({
            "doc_id": parent_id,
            "chunk_index": chunk.index,
            "chunk_total": chunk.total,
            "start_char": chunk.start_char,
            "end_char": chunk.end_char
        });

        // Set text and embedding fields dynamically
        if let Some(obj) = doc.as_object_mut() {
            obj.insert(text_field.clone(), json!(chunk.text));
            obj.insert(embedding_field.clone(), json!(embedding));
        }

        if let Some(ref title) = p.title {
            doc["title"] = json!(title);
        }
        if let Some(ref heading) = chunk.heading {
            doc["section"] = json!(heading);
        }
        if let Some(level) = chunk.heading_level {
            doc["heading_level"] = json!(level);
        }
        if let Some(ref path) = chunk.section_path {
            doc["section_path"] = json!(path);
        }
        // Add custom metadata (with security filtering)
        if let Some(ref metadata) = p.metadata {
            if let Some(meta_obj) = metadata.as_object() {
                if let Some(doc_obj) = doc.as_object_mut() {
                    for (k, v) in meta_obj {
                        // SECURITY: Skip reserved keys to prevent injection
                        if RESERVED_METADATA_KEYS.contains(&k.as_str()) {
                            tracing::warn!(
                                "Ignoring reserved metadata key '{}' in rag_document_import",
                                k
                            );
                            continue;
                        }
                        // Also skip embedding and text fields
                        if k == &embedding_field || k == &text_field {
                            tracing::warn!(
                                "Ignoring protected field '{}' in rag_document_import metadata",
                                k
                            );
                            continue;
                        }
                        doc_obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        documents.push(doc);
    }

    // Validate embedding dimension matches expected (if RAG config exists)
    if let Some(ref cfg) = rag_config {
        let expected_dim = cfg.dimension;
        let actual_dim = provider.dimension();
        if expected_dim != actual_dim {
            return Err(McpError::invalid_params(format!(
                "Embedding dimension mismatch: collection '{}' expects {} dimensions (from RAG config), but provider '{}' produces {} dimensions",
                p.collection, expected_dim, provider_name, actual_dim
            )));
        }
    }

    // Insert documents
    let inserted_ids = adapter.insert_many(&p.collection, documents)?;

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "doc_id": parent_id,
        "chunks_created": inserted_ids.len(),
        "dimension": provider.dimension(),
        "provider": provider_name
    }))
}

// ============================================================================
// rag_search Handler - THE KEY TOOL!
// ============================================================================

/// Intermediate result structure for pipeline processing
#[derive(Debug)]
struct FusedResult {
    #[allow(dead_code)]
    id: String,
    doc: Value,
    rrf_score: f64,
    final_score: f64,
    rerank_boost: f64,
    v_rank: usize,
    t_rank: usize,
    v_score: Option<f32>,
    t_score: Option<f64>,
}

fn handle_rag_search(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    if params.get("dedup_threshold").is_some() {
        return Err(McpError::invalid_params(
            "Parameter 'dedup_threshold' has been removed. Use 'mmr_lambda' (0.0-1.0) instead.",
        ));
    }
    let p: RagSearchParams = RagSearchParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    if p.query.is_empty() {
        return Err(McpError::invalid_params("Query cannot be empty"));
    }

    // Get embedding manager
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
        )
    })?;

    // Get RAG config or use defaults
    let rag_config = get_rag_config(adapter, &p.collection)?;
    let (embedding_field, text_field, provider_name) = match &rag_config {
        Some(cfg) => (
            cfg.embedding_field.clone(),
            cfg.text_field.clone(),
            p.provider.clone().unwrap_or_else(|| cfg.provider.clone()),
        ),
        None => (
            DEFAULT_EMBEDDING_FIELD.to_string(),
            DEFAULT_TEXT_FIELD.to_string(),
            p.provider
                .clone()
                .unwrap_or_else(|| DEFAULT_EMBEDDING_PROVIDER.to_string()),
        ),
    };

    // ========================================================================
    // STEP 1: AUTO-EMBED THE QUERY (THE KEY DIFFERENCE FROM hybrid_search!)
    // ========================================================================
    let query_vector = manager
        .embed(&p.query, Some(&provider_name))
        .map_err(|e| McpError::internal(format!("Query embedding failed: {}", e)))?;

    // Internal limit multiplier for better fusion coverage
    let internal_limit = (p.limit * 3).min(MAX_INTERNAL_LIMIT);

    // ========================================================================
    // STEP 2: Vector search
    // ========================================================================
    let vector_results = if let Some(ref filter) = p.filter {
        adapter.vector_search_with_filter(
            &p.collection,
            &embedding_field,
            &query_vector,
            filter,
            internal_limit,
        )?
    } else {
        adapter.vector_search(
            &p.collection,
            &embedding_field,
            &query_vector,
            internal_limit,
        )?
    };

    // Build vector rank map (1-indexed)
    let mut vector_ranks: HashMap<String, usize> = HashMap::with_capacity(vector_results.len());
    for (rank, (doc, _score)) in vector_results.iter().enumerate() {
        if let Some(id) = doc.get("_id").and_then(id_to_string) {
            vector_ranks.insert(id, rank + 1);
        }
    }

    // Store vector docs for later retrieval
    let mut vector_docs: HashMap<String, (Value, f32)> =
        HashMap::with_capacity(vector_results.len());
    for (doc, score) in vector_results.into_iter() {
        if let Some(id) = doc.get("_id").and_then(id_to_string) {
            vector_docs.insert(id, (doc, score));
        }
    }

    // ========================================================================
    // STEP 3: Fulltext search
    // ========================================================================
    let text_options = FulltextSearchOptions {
        limit: Some(internal_limit),
        skip: None,
        min_score: None,
        projection: None,
        filter: p.filter.clone(),
        and_mode: false, // RAG uses RRF fusion, not AND mode
        highlight: false,
        highlight_context: None,
        highlight_max_snippets: None,
    };

    let text_results =
        adapter.fulltext_search(&p.collection, &text_field, &p.query, text_options)?;

    // Build fulltext rank map (1-indexed)
    let mut text_ranks: HashMap<String, usize> = HashMap::with_capacity(text_results.len());
    for (rank, res) in text_results.iter().enumerate() {
        if let Some(id) = res.document.get("_id").and_then(id_to_string) {
            text_ranks.insert(id, rank + 1);
        }
    }

    // Store fulltext docs for later retrieval
    let mut text_docs: HashMap<String, (Value, f64)> = HashMap::with_capacity(text_results.len());
    for res in text_results.into_iter() {
        if let Some(id) = res.document.get("_id").and_then(id_to_string) {
            text_docs.insert(id, (res.document, res.score));
        }
    }

    // ========================================================================
    // STEP 4: RRF Fusion
    // ========================================================================
    let max_ids = vector_ranks.len() + text_ranks.len();
    let mut all_ids: HashSet<String> = HashSet::with_capacity(max_ids);
    all_ids.extend(vector_ranks.keys().cloned());
    all_ids.extend(text_ranks.keys().cloned());

    let default_rank = internal_limit + 1;
    let mut fused: Vec<FusedResult> = Vec::with_capacity(all_ids.len());

    // Normalize weights to ensure they sum to 1.0 for consistent RRF scoring
    let total_weight = p.vector_weight + p.fulltext_weight;
    let norm_vector_weight = if total_weight > 0.0 {
        p.vector_weight / total_weight
    } else {
        0.5
    };
    let norm_fulltext_weight = if total_weight > 0.0 {
        p.fulltext_weight / total_weight
    } else {
        0.5
    };

    for id in all_ids.iter() {
        let v_rank = *vector_ranks.get(id).unwrap_or(&default_rank);
        let t_rank = *text_ranks.get(id).unwrap_or(&default_rank);

        // RRF score formula with normalized weights: weight * 1/(k + rank)
        let rrf_score = norm_vector_weight * (1.0 / (RRF_K + v_rank as f64))
            + norm_fulltext_weight * (1.0 / (RRF_K + t_rank as f64));

        let v_score = vector_docs.get(id).map(|(_, s)| *s);
        let t_score = text_docs.get(id).map(|(_, s)| *s);

        // Get document from either source (prefer vector for consistency)
        let doc = match vector_docs
            .get(id)
            .map(|(d, _)| d.clone())
            .or_else(|| text_docs.get(id).map(|(d, _)| d.clone()))
        {
            Some(d) => d,
            None => continue,
        };

        fused.push(FusedResult {
            id: id.clone(),
            doc,
            rrf_score,
            final_score: rrf_score,
            rerank_boost: 1.0,
            v_rank,
            t_rank,
            v_score,
            t_score,
        });
    }

    // Sort by RRF score initially
    fused.sort_by(|a, b| {
        b.rrf_score
            .partial_cmp(&a.rrf_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ========================================================================
    // STEP 5: Reranking (optional)
    // ========================================================================
    if p.rerank {
        rerank_results(&mut fused, &p.query, &text_field);
    }

    // ========================================================================
    // STEP 6: MMR diversity reranking (optional) — replaces prefix-based dedup
    // ========================================================================
    let dedup_removed = if p.deduplicate {
        mmr_reorder(&mut fused, &embedding_field, p.mmr_lambda, p.limit)
    } else {
        fused.truncate(p.limit);
        0
    };

    // ========================================================================
    // STEP 7: Apply projection and build response
    // ========================================================================
    let projection = parse_projection_value(p.projection)?;

    // Pre-allocate with try_reserve for OOM protection
    let mut results: Vec<Value> = Vec::new();
    results.try_reserve(fused.len()).map_err(|e| {
        McpError::internal(format!(
            "OOM: cannot allocate {} results: {}",
            fused.len(),
            e
        ))
    })?;

    for item in fused {
        let doc_projected = if let Some(ref proj) = projection {
            apply_projection(&item.doc, proj)
        } else {
            item.doc
        };

        let mut result = doc_projected;
        if let Value::Object(ref mut obj) = result {
            obj.insert("_rrf_score".to_string(), json!(item.rrf_score));
            obj.insert("_final_score".to_string(), json!(item.final_score));
            obj.insert("_rerank_boost".to_string(), json!(item.rerank_boost));
            obj.insert("_vector_rank".to_string(), json!(item.v_rank));
            obj.insert("_text_rank".to_string(), json!(item.t_rank));
            if let Some(vs) = item.v_score {
                obj.insert("_vector_score".to_string(), json!(vs));
            }
            if let Some(ts) = item.t_score {
                obj.insert("_text_score".to_string(), json!(ts));
            }
        }

        results.push(result);
    }

    Ok(json!({
        "results": results,
        "count": results.len(),
        "query": p.query,
        "algorithm": "rag_hybrid_rrf",
        "rrf_k": RRF_K,
        "weights": {
            "vector": p.vector_weight,
            "fulltext": p.fulltext_weight
        },
        "provider": provider_name,
        "dedup_removed": dedup_removed
    }))
}

// ============================================================================
// rag_collection_stats Handler
// ============================================================================

fn handle_rag_collection_stats(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: RagCollectionStatsParams = RagCollectionStatsParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Get RAG config
    let rag_config = get_rag_config(adapter, &p.collection)?;

    // Get document count (chunks)
    let chunk_count = adapter
        .count_documents(&p.collection, json!({}))
        .unwrap_or(0);

    // Get unique doc_ids (source documents)
    let source_doc_count = adapter
        .distinct(&p.collection, "doc_id", json!({}))
        .map(|v| v.len())
        .unwrap_or(0);

    // Get vector indexes
    let vector_indexes = adapter
        .list_vector_indexes(&p.collection)
        .unwrap_or_default();

    // Get fulltext indexes
    let fulltext_indexes = adapter
        .list_fulltext_indexes(&p.collection)
        .unwrap_or_default();

    // Provider info
    let provider_info = if let (Some(cfg), Some(mgr)) = (&rag_config, embedding_manager.as_ref()) {
        mgr.get_provider(&cfg.provider).map(|p| {
            json!({
                "name": cfg.provider,
                "dimension": p.dimension(),
                "model": p.model_name()
            })
        })
    } else {
        None
    };

    Ok(json!({
        "collection": p.collection,
        "rag_enabled": rag_config.is_some(),
        "config": rag_config.map(|c| json!({
            "embedding_field": c.embedding_field,
            "text_field": c.text_field,
            "provider": c.provider,
            "language": c.language,
            "dimension": c.dimension,
            "created_at": c.created_at
        })),
        "stats": {
            "chunk_count": chunk_count,
            "source_document_count": source_doc_count,
            "vector_indexes": vector_indexes,
            "fulltext_indexes": fulltext_indexes
        },
        "provider_info": provider_info
    }))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert Value _id to String for HashMap key
fn id_to_string(id: &Value) -> Option<String> {
    match id {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => Some(id.to_string()),
    }
}

/// Strip punctuation for phrase matching (optimized: single allocation)
fn strip_punctuation(s: &str) -> String {
    let filtered: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    // Normalize whitespace in-place without extra Vec allocation
    let mut result = String::with_capacity(filtered.len());
    let mut prev_space = true; // Start true to skip leading spaces
    for c in filtered.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Rerank results by phrase match, keyword density, and content length
fn rerank_results(results: &mut [FusedResult], query: &str, text_field: &str) {
    // Build query word sets (filter words with at least 3 bytes - fast approximation)
    // Note: 3 bytes = at least 1-3 UTF-8 chars, good enough for filtering short words
    let query_words: HashSet<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|s| s.to_string())
        .collect();

    let query_normalized = strip_punctuation(&query.to_lowercase());

    for item in results.iter_mut() {
        let mut boost = 1.0;

        let content = item
            .doc
            .get(text_field)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let content_lower = content.to_lowercase();
        let content_normalized = strip_punctuation(&content_lower);

        // 1. Exact phrase boost (1.3x)
        if query_normalized.chars().count() > 10 && content_normalized.contains(&query_normalized) {
            boost *= 1.3;
        }

        // 2. Keyword density (1.0-1.1x)
        let content_words: Vec<&str> = content_lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect();

        if !content_words.is_empty() {
            #[allow(clippy::unnecessary_to_owned)]
            let matches = content_words
                .iter()
                .filter(|w| query_words.contains(&w.to_string()))
                .count();
            let density = matches as f64 / content_words.len() as f64;
            boost *= 1.0 + density.min(0.1);
        }

        // 3. Content length penalty (0.8x for short content)
        if content.len() < 100 {
            boost *= 0.8;
        }

        item.rerank_boost = boost;
        item.final_score = item.rrf_score * boost;
    }

    // Re-sort by final_score
    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Extract embedding vector from a document's embedding field (JSON array → Vec<f32>)
fn extract_embedding(doc: &Value, embedding_field: &str) -> Option<Vec<f32>> {
    doc.get(embedding_field)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
}

/// MMR reordering: selects `limit` results balancing relevance and diversity.
///
/// Algorithm: greedily selects the candidate that maximizes:
///   mmr(c) = λ * relevance(c) - (1-λ) * max_sim(c, selected)
///
/// where relevance is the normalized final_score and max_sim is the maximum
/// cosine similarity between c's embedding and any already-selected result.
///
/// Results without embeddings are kept in relevance order (no diversity penalty).
///
/// Returns the number of candidates not selected (analogous to dedup_removed).
fn mmr_reorder(
    results: &mut Vec<FusedResult>,
    embedding_field: &str,
    lambda: f64,
    limit: usize,
) -> usize {
    let original_len = results.len();
    if results.is_empty() || limit == 0 {
        results.clear();
        return original_len;
    }

    let target = limit.min(original_len);

    // Extract embeddings (None if missing from doc)
    let embeddings: Vec<Option<Vec<f32>>> = results
        .iter()
        .map(|r| extract_embedding(&r.doc, embedding_field))
        .collect();

    // Normalize relevance scores to [0, 1] for MMR formula
    let max_score = results
        .iter()
        .map(|r| r.final_score)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = results
        .iter()
        .map(|r| r.final_score)
        .fold(f64::INFINITY, f64::min);
    let score_range = max_score - min_score;

    // Track which indices are selected and which are candidates
    let mut selected_indices: Vec<usize> = Vec::with_capacity(target);
    let mut candidate_mask: Vec<bool> = vec![true; original_len];

    // Select first: highest relevance (results are pre-sorted by final_score)
    selected_indices.push(0);
    candidate_mask[0] = false;

    // Greedy MMR selection
    while selected_indices.len() < target {
        let mut best_idx = None;
        let mut best_mmr = f64::NEG_INFINITY;

        for (i, is_candidate) in candidate_mask.iter().enumerate() {
            if !is_candidate {
                continue;
            }

            // Normalized relevance [0, 1]
            let relevance = if score_range > 0.0 {
                (results[i].final_score - min_score) / score_range
            } else {
                1.0
            };

            // Max similarity to any selected result
            let max_sim = if let Some(ref emb_i) = embeddings[i] {
                selected_indices
                    .iter()
                    .filter_map(|&si| {
                        embeddings[si].as_ref().map(|emb_s| {
                            if emb_i.len() == emb_s.len() && !emb_i.is_empty() {
                                ironbase_core::vector::simd::cosine_similarity(emb_i, emb_s)
                                    as f64
                            } else {
                                0.0 // Dimension mismatch → no penalty
                            }
                        })
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
            } else {
                0.0 // No embedding → no diversity penalty
            };
            let max_sim = if max_sim == f64::NEG_INFINITY {
                0.0
            } else {
                max_sim
            };

            let mmr_score = lambda * relevance - (1.0 - lambda) * max_sim;

            if mmr_score > best_mmr {
                best_mmr = mmr_score;
                best_idx = Some(i);
            }
        }

        match best_idx {
            Some(idx) => {
                selected_indices.push(idx);
                candidate_mask[idx] = false;
            }
            None => break, // No more candidates
        }
    }

    // Reorder results: keep only selected, in selection order
    let reordered: Vec<FusedResult> = selected_indices
        .into_iter()
        .map(|i| {
            let mut placeholder = FusedResult {
                id: String::new(),
                doc: Value::Null,
                rrf_score: 0.0,
                final_score: 0.0,
                rerank_boost: 0.0,
                v_rank: 0,
                t_rank: 0,
                v_score: None,
                t_score: None,
            };
            std::mem::swap(&mut results[i], &mut placeholder);
            placeholder
        })
        .collect();

    *results = reordered;
    original_len - results.len()
}

/// Apply MongoDB-style projection to a document
fn apply_projection(doc: &Value, projection: &HashMap<String, i32>) -> Value {
    let is_include_mode = projection.values().any(|&v| v == 1);
    let id_explicitly_excluded = projection.get("_id").copied() == Some(0);

    let mut result = json!({});
    if let Value::Object(obj) = doc {
        for (key, value) in obj {
            let should_include = if is_include_mode {
                if key == "_id" {
                    !id_explicitly_excluded
                } else {
                    projection.get(key).copied().unwrap_or(0) == 1
                }
            } else {
                projection.get(key).copied().unwrap_or(1) != 0
            };
            if should_include {
                result[key] = value.clone();
            }
        }
    }
    result
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rag_config_serialization() {
        let config = RagConfig {
            collection: "test".to_string(),
            embedding_field: "embedding".to_string(),
            text_field: "content".to_string(),
            provider: "fasttext".to_string(),
            language: "hungarian".to_string(),
            dimension: 300,
            created_at: "2026-01-27T12:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&config).unwrap();
        let parsed: RagConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.collection, "test");
        assert_eq!(parsed.dimension, 300);
    }

    #[test]
    fn test_strip_punctuation() {
        assert_eq!(strip_punctuation("hello, world!"), "hello world");
        assert_eq!(strip_punctuation("a. b. c."), "a b c");
        assert_eq!(strip_punctuation("király   mesék"), "király mesék");
    }

    #[test]
    fn test_id_to_string() {
        assert_eq!(id_to_string(&json!("abc")), Some("abc".to_string()));
        assert_eq!(id_to_string(&json!(123)), Some("123".to_string()));
    }

    #[test]
    fn test_rag_search_params_defaults() {
        let params = json!({
            "collection": "docs",
            "query": "semantic search test"
        });
        let p: RagSearchParams = RagSearchParams::parse(params).unwrap();
        assert_eq!(p.collection, "docs");
        assert_eq!(p.query, "semantic search test");
        assert_eq!(p.limit, 10);
        assert_eq!(p.vector_weight, 0.5);
        assert_eq!(p.fulltext_weight, 0.5);
        assert!(p.rerank);
        assert!(p.deduplicate);
        assert!((p.mmr_lambda - 0.5).abs() < f64::EPSILON);
        assert!(p.projection.is_none());
        assert!(p.provider.is_none());
        assert!(p.filter.is_none()); // default: no filter
    }

    #[test]
    fn test_rag_search_params_with_filter() {
        let params = json!({
            "collection": "docs",
            "query": "keresés",
            "filter": {"doc_type": "ajanlat", "year": 2026}
        });
        let p: RagSearchParams = RagSearchParams::parse(params).unwrap();
        assert!(p.filter.is_some());
        let filter = p.filter.unwrap();
        assert_eq!(filter["doc_type"], "ajanlat");
        assert_eq!(filter["year"], 2026);
    }

    #[test]
    fn test_rag_search_params_with_empty_filter() {
        let params = json!({
            "collection": "docs",
            "query": "test",
            "filter": {}
        });
        let p: RagSearchParams = RagSearchParams::parse(params).unwrap();
        assert!(p.filter.is_some());
        assert!(p.filter.unwrap().as_object().unwrap().is_empty());
    }
}
