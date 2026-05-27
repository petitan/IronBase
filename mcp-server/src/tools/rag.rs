//! RAG (Retrieval-Augmented Generation) tool handlers
//!
//! Provides MCP tools for RAG collection management:
//! - rag_collection_create: Set up collection with indexes
//! - rag_document_import: Chunk + embed + insert
//! - rag_collection_stats: RAG-specific statistics
//!
//! Search logic lives in hybrid_search (see hybrid.rs).

use crate::adapter::IronBaseAdapter;
use crate::chunking::{build_embed_text, chunk_content, ChunkMode, ChunkOptions};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD};
use super::helpers::{
    insert_chunks_idempotent, should_skip_before_embedding, validate_collection_name, IfExists,
    RESERVED_METADATA_KEYS,
};
use super::params::{
    ParseParams, RagCollectionCreateParams, RagCollectionStatsParams, RagDocumentImportParams,
};

/// System collection for RAG configs
const RAG_CONFIG_COLLECTION: &str = "_system.rag";

// ============================================================================
// RAG Config Storage
// ============================================================================

/// RAG configuration stored in _system.rag collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub collection: String,
    pub embedding_field: String,
    pub text_field: String,
    /// All fulltext-indexed text fields (primary + extras). Empty on legacy
    /// configs → callers fall back to `[text_field]`. Lets hybrid_search default
    /// to multi-field search consistently with how the collection was set up (#66).
    #[serde(default)]
    pub text_fields: Vec<String>,
    pub provider: String,
    pub language: String,
    pub dimension: usize,
    pub created_at: String,
}

impl RagConfig {
    /// The effective list of fulltext fields, falling back to the single
    /// `text_field` for legacy configs that predate `text_fields`.
    pub fn effective_text_fields(&self) -> Vec<String> {
        if self.text_fields.is_empty() {
            vec![self.text_field.clone()]
        } else {
            self.text_fields.clone()
        }
    }
}

/// Get RAG config for a collection from _system.rag
pub(crate) fn get_rag_config(
    adapter: &IronBaseAdapter,
    collection: &str,
) -> Result<Option<RagConfig>> {
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
            "Embedding not available. Configure an [embedding] section in config.toml.",
        )
    })?;

    // Resolve provider: explicit param, else manager default
    let provider_name = p
        .provider
        .as_deref()
        .unwrap_or(manager.default_provider_name())
        .to_string();

    if provider_name.is_empty() {
        return Err(McpError::invalid_params(
            "No embedding provider available. Configure an [embedding] section in config.toml \
             or pass an explicit 'provider' argument.",
        ));
    }

    // Validate provider exists
    let provider = manager.get_provider(&provider_name).ok_or_else(|| {
        let available: Vec<_> = manager
            .list_models()
            .iter()
            .map(|m| m.provider.clone())
            .collect();
        McpError::invalid_params(format!(
            "Provider '{}' not found. Available: {:?}",
            provider_name, available
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

    // 3. Create a fulltext index on every requested text field (the primary
    //    text_field is always included), so multi-field hybrid_search works
    //    without manual index_create_fulltext calls (#66).
    let fulltext_fields = resolve_fulltext_fields(&p.text_field, &p.text_fields);
    let mut fulltext_created = 0usize;
    for field in &fulltext_fields {
        match adapter.create_fulltext_index(&p.collection, field, &p.language, Some(2), Some(true))
        {
            Ok(_) => fulltext_created += 1,
            Err(e) => {
                tracing::debug!("Fulltext index on '{}': {} (may already exist)", field, e)
            }
        }
    }

    // 4. Save RAG config
    let config = RagConfig {
        collection: p.collection.clone(),
        embedding_field: p.embedding_field.clone(),
        text_field: p.text_field.clone(),
        text_fields: fulltext_fields.clone(),
        provider: provider_name.clone(),
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
            "text_fields": fulltext_fields,
            "provider": provider_name,
            "language": p.language,
            "dimension": dimension
        },
        "indexes": {
            "collection_created": collection_created,
            "vector_created": vector_index_created,
            "fulltext_created": fulltext_created
        }
    }))
}

/// Resolve which fulltext fields a hybrid_search should query (#66).
///
/// - An explicit, non-empty `explicit` list (the caller's `text_fields`) wins.
/// - Otherwise fall back to the collection's configured `config_fields`,
///   intersected with the fields that ACTUALLY have a fulltext index — a
///   configured-but-unindexed field (failed creation / later `index_drop`) must
///   never reach `fulltext_search_multi`, which hard-errors on a missing index.
/// - Returns `None` for a single (or empty) field set → callers use single-field
///   search on the primary text field.
pub(crate) fn resolve_search_text_fields(
    explicit: Option<Vec<String>>,
    config_fields: Vec<String>,
    indexed: &[String],
) -> Option<Vec<String>> {
    if let Some(tf) = explicit {
        if !tf.is_empty() {
            return Some(tf);
        }
    }
    let resolved: Vec<String> = config_fields
        .into_iter()
        .filter(|f| indexed.iter().any(|i| i == f))
        .collect();
    if resolved.len() > 1 {
        Some(resolved)
    } else {
        None
    }
}

/// Resolve the full set of fulltext fields to index: the explicit `text_fields`
/// list if given, always with the primary `text_field` included, deduplicated
/// in order. Omitted/empty → just `[text_field]`.
pub(crate) fn resolve_fulltext_fields(
    text_field: &str,
    text_fields: &Option<Vec<String>>,
) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    fields.push(text_field.to_string());
    if let Some(extra) = text_fields {
        for f in extra {
            if !f.is_empty() && !fields.iter().any(|e| e == f) {
                fields.push(f.clone());
            }
        }
    }
    fields
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
            "Embedding not available. Configure an [embedding] section in config.toml.",
        )
    })?;

    // Resolve provider: user explicit > AutoEmbeddingConfig > RAG config > manager default
    let auto_provider = adapter
        .get_auto_embedding_config(&p.collection)
        .ok()
        .flatten()
        .map(|c| c.provider);

    let rag_config = get_rag_config(adapter, &p.collection)?;
    let (embedding_field, text_field, provider_name) = match &rag_config {
        Some(cfg) => (
            cfg.embedding_field.clone(),
            cfg.text_field.clone(),
            p.provider
                .clone()
                .or(auto_provider)
                .unwrap_or_else(|| cfg.provider.clone()),
        ),
        None => (
            DEFAULT_EMBEDDING_FIELD.to_string(),
            DEFAULT_TEXT_FIELD.to_string(),
            p.provider
                .clone()
                .or(auto_provider)
                .unwrap_or_else(|| manager.default_provider_name().to_string()),
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

    // Pre-check before the expensive embedding step: skip/error short-circuit if
    // an explicit doc_id already has chunks (auto-generated ids never collide).
    let if_exists = IfExists::parse(&p.if_exists);
    if let Some(ref doc_id) = p.doc_id {
        if should_skip_before_embedding(adapter, &p.collection, doc_id, if_exists)? {
            return Ok(json!({
                "success": true,
                "collection": p.collection,
                "doc_id": doc_id,
                "chunks_created": 0,
                "skipped": true,
                "message": "doc_id already exists; skipped (if_exists=skip)"
            }));
        }
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
        // Embed breadcrumb + cleaned body; the original chunk.text is stored unchanged.
        let texts: Vec<String> = batch
            .iter()
            .map(|c| build_embed_text(&c.text, c.section_path.as_deref()))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = provider
            .embed_batch(&text_refs)
            .map_err(|e| McpError::internal(format!("Embedding failed: {}", e)))?;
        all_embeddings.extend(embeddings);
    }

    // Ensure collection and indexes exist if no RAG config (idempotent)
    let mut language_ignored = false;
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
        // Fulltext index on every requested field (#66), and persist a RAG config
        // so later hybrid_search defaults to multi-field search consistently.
        let fulltext_fields = resolve_fulltext_fields(&text_field, &p.text_fields);
        for field in &fulltext_fields {
            let _ = adapter.create_fulltext_index(
                &p.collection,
                field,
                &p.language,
                Some(2),
                Some(true),
            );
        }
        let config = RagConfig {
            collection: p.collection.clone(),
            embedding_field: embedding_field.clone(),
            text_field: text_field.clone(),
            text_fields: fulltext_fields,
            provider: provider_name.clone(),
            language: p.language.clone(),
            dimension: provider.dimension(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        save_rag_config(adapter, &config)?;
    } else if let Some(ref cfg) = rag_config {
        // The collection already has indexes/config; index-shaping params here are
        // no-ops. Only warn when the request actually DIFFERS from the stored
        // config (avoids a false positive on repeated imports with the same args).
        if p.language != "none" && p.language != cfg.language {
            tracing::warn!(
                "rag_document_import: 'language={}' ignored — collection '{}' already has a \
                 fulltext index ('{}'). Set the language via rag_collection_create.",
                p.language,
                p.collection,
                cfg.language
            );
            language_ignored = true;
        }
        if let Some(ref requested) = p.text_fields {
            let missing: Vec<&String> = requested
                .iter()
                .filter(|f| !cfg.effective_text_fields().iter().any(|e| e == *f))
                .collect();
            if !missing.is_empty() {
                tracing::warn!(
                    "rag_document_import: 'text_fields' {:?} ignored — collection '{}' already \
                     has a RAG config. Set text_fields via rag_collection_create.",
                    missing,
                    p.collection
                );
            }
        }
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
        if let Some(ref th) = chunk.table_header {
            doc["table_header"] = json!(th);
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

    // Insert documents (idempotent w.r.t. doc_id — see issue #67).
    // if_exists was parsed before embedding (skip/error already short-circuited).
    let result =
        insert_chunks_idempotent(adapter, &p.collection, &parent_id, documents, if_exists)?;

    if result.skipped {
        return Ok(json!({
            "success": true,
            "collection": p.collection,
            "doc_id": parent_id,
            "chunks_created": 0,
            "skipped": true,
            "message": "doc_id already exists; skipped (if_exists=skip)"
        }));
    }

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "doc_id": parent_id,
        "chunks_created": result.inserted_ids.len(),
        "chunks_replaced": result.replaced,
        "dimension": provider.dimension(),
        "provider": provider_name,
        "language_ignored": language_ignored
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
            "text_fields": c.effective_text_fields(),
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
            text_fields: vec!["content".to_string(), "title".to_string()],
            provider: "ollama".to_string(),
            language: "hungarian".to_string(),
            dimension: 300,
            created_at: "2026-01-27T12:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&config).unwrap();
        let parsed: RagConfig = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.collection, "test");
        assert_eq!(parsed.dimension, 300);
        assert_eq!(parsed.text_fields, vec!["content", "title"]);
    }

    #[test]
    fn test_legacy_config_without_text_fields_falls_back() {
        // A pre-#66 stored config has no text_fields → deserializes to empty,
        // and effective_text_fields falls back to [text_field].
        let json = json!({
            "collection": "c", "embedding_field": "embedding", "text_field": "content",
            "provider": "ollama", "language": "none", "dimension": 384, "created_at": "x"
        });
        let cfg: RagConfig = serde_json::from_value(json).unwrap();
        assert!(cfg.text_fields.is_empty());
        assert_eq!(cfg.effective_text_fields(), vec!["content"]);
    }

    #[test]
    fn test_resolve_search_text_fields() {
        let indexed = vec!["content".to_string(), "title".to_string()];
        // Explicit non-empty wins (even if not all indexed — caller's choice).
        assert_eq!(
            resolve_search_text_fields(Some(vec!["a".into(), "b".into()]), vec![], &indexed),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        // Explicit empty → fall back to config.
        assert_eq!(
            resolve_search_text_fields(
                Some(vec![]),
                vec!["content".into(), "title".into()],
                &indexed
            ),
            Some(vec!["content".to_string(), "title".to_string()])
        );
        // Config field WITHOUT an index is dropped (would hard-error multi-search).
        assert_eq!(
            resolve_search_text_fields(
                None,
                vec!["content".into(), "title".into(), "customer".into()],
                &indexed
            ),
            Some(vec!["content".to_string(), "title".to_string()])
        );
        // After intersection only one field remains → None (single-field search).
        assert_eq!(
            resolve_search_text_fields(None, vec!["content".into(), "ghost".into()], &indexed),
            None
        );
        // No config → None.
        assert_eq!(resolve_search_text_fields(None, vec![], &indexed), None);
    }

    #[test]
    fn test_resolve_fulltext_fields() {
        let tf = "content".to_string();
        // No extras → just the primary.
        assert_eq!(resolve_fulltext_fields(&tf, &None), vec!["content"]);
        // Extras → primary first, then extras, deduplicated.
        assert_eq!(
            resolve_fulltext_fields(&tf, &Some(vec!["title".into(), "customer".into()])),
            vec!["content", "title", "customer"]
        );
        // Primary already in the list → not duplicated.
        assert_eq!(
            resolve_fulltext_fields(&tf, &Some(vec!["content".into(), "title".into()])),
            vec!["content", "title"]
        );
        // Empty strings skipped.
        assert_eq!(
            resolve_fulltext_fields(&tf, &Some(vec!["".into(), "title".into()])),
            vec!["content", "title"]
        );
    }
}
