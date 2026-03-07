//! RAG (Retrieval-Augmented Generation) tool handlers
//!
//! Provides MCP tools for RAG collection management:
//! - rag_collection_create: Set up collection with indexes
//! - rag_document_import: Chunk + embed + insert
//! - rag_collection_stats: RAG-specific statistics
//!
//! Search logic lives in hybrid_search (see hybrid.rs).

use crate::adapter::IronBaseAdapter;
use crate::chunking::{chunk_content, ChunkMode, ChunkOptions};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use super::defaults::{DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD};
use super::helpers::validate_collection_name;
use super::params::{
    ParseParams, RagCollectionCreateParams, RagCollectionStatsParams, RagDocumentImportParams,
};

/// System collection for RAG configs
const RAG_CONFIG_COLLECTION: &str = "_system.rag";

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
        Some(cfg) => {
            let auto_provider = adapter
                .get_auto_embedding_config(&p.collection)
                .ok()
                .flatten()
                .map(|c| c.provider);
            (
                cfg.embedding_field.clone(),
                cfg.text_field.clone(),
                p.provider.clone()
                    .or(auto_provider)
                    .unwrap_or_else(|| cfg.provider.clone()),
            )
        }
        None => {
            let auto_provider = adapter
                .get_auto_embedding_config(&p.collection)
                .ok()
                .flatten()
                .map(|c| c.provider);
            (
                DEFAULT_EMBEDDING_FIELD.to_string(),
                DEFAULT_TEXT_FIELD.to_string(),
                p.provider
                    .clone()
                    .or(auto_provider)
                    .unwrap_or_else(|| manager.default_provider_name().to_string()),
            )
        }
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
}
