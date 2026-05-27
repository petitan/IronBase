//! Embedding generation tool handlers

use crate::adapter::IronBaseAdapter;
use crate::chunking::{build_embed_text, chunk_content, ChunkMode, ChunkOptions};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::defaults::{default_chunk_mode, default_chunk_overlap, default_chunk_size};
use super::helpers::{
    insert_chunks_idempotent, should_skip_before_embedding, IfExists, RESERVED_METADATA_KEYS,
};
use super::params::ParseParams;

// ============================================================================
// Parameter Structs
// ============================================================================

/// Parameters for `embed_text` tool
#[derive(Debug, Deserialize)]
pub struct EmbedTextParams {
    pub text: String,
    pub provider: Option<String>,
}

/// Parameters for `embed_batch` tool
#[derive(Debug, Deserialize)]
pub struct EmbedBatchParams {
    pub texts: Vec<String>,
    pub provider: Option<String>,
}

/// Parameters for `embed_list_models` tool (no params needed)
#[derive(Debug, Deserialize)]
pub struct EmbedListModelsParams {}

/// Parameters for `embed_document` tool
#[derive(Debug, Deserialize)]
pub struct EmbedDocumentParams {
    pub collection: String,
    pub content: String,
    pub doc_id: Option<String>,
    pub title: Option<String>,
    pub metadata: Option<Value>,
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
    pub provider: Option<String>,
    #[serde(default = "default_create_vector_index")]
    pub create_vector_index: bool,
    #[serde(default = "crate::tools::helpers::default_if_exists")]
    pub if_exists: String,
}

fn default_create_vector_index() -> bool {
    true
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch embedding tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    adapter: Option<&Arc<IronBaseAdapter>>,
) -> Result<Value> {
    // Check if embedding manager is available
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Configure an [embedding] section in config.toml.",
        )
    })?;

    match name {
        "embed_text" => handle_embed_text(params, manager),
        "embed_batch" => handle_embed_batch(params, manager),
        "embed_list_models" => handle_embed_list_models(manager),
        "embed_document" => {
            let adapter = adapter.ok_or_else(|| {
                McpError::internal("Database adapter not available for embed_document")
            })?;
            handle_embed_document(params, manager, adapter)
        }
        "embed_cache_stats" => handle_embed_cache_stats(manager),
        "embed_cache_clear" => handle_embed_cache_clear(manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown embedding tool: {}",
            name
        ))),
    }
}

/// Check if a tool name is an embedding tool
pub fn is_embedding_tool(name: &str) -> bool {
    matches!(
        name,
        "embed_text"
            | "embed_batch"
            | "embed_list_models"
            | "embed_document"
            | "embed_cache_stats"
            | "embed_cache_clear"
    )
}

// ============================================================================
// Tool Handlers
// ============================================================================

fn handle_embed_text(params: Value, manager: &EmbeddingManager) -> Result<Value> {
    let p: EmbedTextParams = EmbedTextParams::parse(params)?;

    let provider_name = resolve_provider(p.provider.as_deref(), manager)?;

    let vector = manager
        .embed(&p.text, Some(&provider_name))
        .map_err(|e| McpError::internal(format!("Embedding failed: {}", e)))?;

    Ok(json!({
        "vector": vector,
        "dimension": vector.len(),
        "provider": provider_name,
        "model": manager.get_provider(&provider_name)
            .map(|p| p.model_name().to_string())
            .unwrap_or_default()
    }))
}

/// Resolve provider: explicit param > manager default. Errors if neither is set.
fn resolve_provider(explicit: Option<&str>, manager: &EmbeddingManager) -> Result<String> {
    let name = explicit
        .unwrap_or(manager.default_provider_name())
        .to_string();
    if name.is_empty() {
        return Err(McpError::invalid_params(
            "No embedding provider available. Configure an [embedding] section in config.toml \
             or pass an explicit 'provider' argument.",
        ));
    }
    Ok(name)
}

/// Maximum total characters allowed in a batch to prevent OOM
const MAX_BATCH_TOTAL_CHARS: usize = 10_000_000; // 10MB

fn handle_embed_batch(params: Value, manager: &EmbeddingManager) -> Result<Value> {
    let p: EmbedBatchParams = EmbedBatchParams::parse(params)?;

    // Validate batch size
    if p.texts.is_empty() {
        return Err(McpError::invalid_params("texts array cannot be empty"));
    }
    if p.texts.len() > 100 {
        return Err(McpError::invalid_params(
            "texts array cannot have more than 100 elements",
        ));
    }

    // Validate total character count to prevent OOM
    let total_chars: usize = p.texts.iter().map(|s| s.len()).sum();
    if total_chars > MAX_BATCH_TOTAL_CHARS {
        return Err(McpError::invalid_params(format!(
            "Total text size {} bytes exceeds limit {} bytes",
            total_chars, MAX_BATCH_TOTAL_CHARS
        )));
    }

    let provider_name = resolve_provider(p.provider.as_deref(), manager)?;

    // Convert Vec<String> to Vec<&str>
    let text_refs: Vec<&str> = p.texts.iter().map(|s| s.as_str()).collect();

    let vectors = manager
        .embed_batch(&text_refs, Some(&provider_name))
        .map_err(|e| McpError::internal(format!("Batch embedding failed: {}", e)))?;

    let dimension = vectors.first().map(|v| v.len()).unwrap_or(0);

    Ok(json!({
        "vectors": vectors,
        "count": vectors.len(),
        "dimension": dimension,
        "provider": provider_name,
        "model": manager.get_provider(&provider_name)
            .map(|p| p.model_name().to_string())
            .unwrap_or_default()
    }))
}

fn handle_embed_list_models(manager: &EmbeddingManager) -> Result<Value> {
    let models = manager.list_models();

    Ok(json!({
        "models": models,
        "count": models.len(),
        "default_provider": manager.default_provider_name()
    }))
}

fn handle_embed_document(
    params: Value,
    manager: &EmbeddingManager,
    adapter: &Arc<IronBaseAdapter>,
) -> Result<Value> {
    let p: EmbedDocumentParams = EmbedDocumentParams::parse(params)?;

    // Validate content
    if p.content.is_empty() {
        return Err(McpError::invalid_params("Content cannot be empty"));
    }

    let provider_name = resolve_provider(p.provider.as_deref(), manager)?;

    let provider = manager.get_provider(&provider_name).ok_or_else(|| {
        McpError::invalid_params(format!("Provider '{}' not available", provider_name))
    })?;

    // Chunk the content
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

    // Generate embeddings in batch (process in chunks of 100)
    let mut all_embeddings: Vec<Vec<f32>> = Vec::new();
    // Pre-allocate to avoid OOM on large documents
    all_embeddings.try_reserve(chunks.len()).map_err(|e| {
        McpError::internal(format!(
            "Cannot allocate memory for {} embeddings: {}",
            chunks.len(),
            e
        ))
    })?;
    let batch_size = 100;

    for batch in chunks.chunks(batch_size) {
        // Embed breadcrumb + cleaned body; the original chunk.text is stored unchanged.
        let texts: Vec<String> = batch
            .iter()
            .map(|c| build_embed_text(&c.text, c.section_path.as_deref()))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = provider
            .embed_batch(&text_refs)
            .map_err(|e| McpError::internal(format!("Embedding generation failed: {}", e)))?;
        all_embeddings.extend(embeddings);
    }

    // Create collection if it doesn't exist
    if let Err(e) = adapter.create_collection(&p.collection) {
        tracing::debug!("Collection creation note: {} (may already exist)", e);
    }

    // Create vector index if requested
    if p.create_vector_index {
        let dimension = provider.dimension();
        // Try to create index with default HNSW parameters, ignore if already exists
        // metric: cosine, max_vectors: 100000, m: 16, ef_construction: 100, ef_search: 50
        if let Err(e) = adapter.create_vector_index(
            &p.collection,
            "embedding",
            dimension,
            "cosine",
            100_000, // max_vectors
            16,      // m
            100,     // ef_construction
            50,      // ef_search
        ) {
            tracing::debug!("Vector index creation note: {} (may already exist)", e);
        }
    }

    // Generate parent doc_id if not provided
    let parent_id = p.doc_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Insert chunks as documents
    let mut documents: Vec<Value> = Vec::new();
    for (chunk, embedding) in chunks.iter().zip(all_embeddings.iter()) {
        let mut doc = json!({
            "doc_id": parent_id,
            "chunk_index": chunk.index,
            "chunk_total": chunk.total,
            "content": chunk.text,
            "embedding": embedding,
            "start_char": chunk.start_char,
            "end_char": chunk.end_char
        });

        // Add optional fields
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

        if let Some(ref metadata) = p.metadata {
            if let Some(obj) = metadata.as_object() {
                if let Some(doc_obj) = doc.as_object_mut() {
                    for (key, value) in obj {
                        // SECURITY/idempotency: never let metadata overwrite chunk-tracking
                        // fields (e.g. doc_id, which drives replace) or the content/embedding.
                        if RESERVED_METADATA_KEYS.contains(&key.as_str())
                            || key == "content"
                            || key == "embedding"
                        {
                            tracing::warn!(
                                "Ignoring protected field '{}' in embed_document metadata",
                                key
                            );
                            continue;
                        }
                        doc_obj.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        documents.push(doc);
    }

    // Insert all documents (idempotent w.r.t. doc_id — see issue #67).
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
        "provider": p.provider,
        "vector_index_created": p.create_vector_index,
        "inserted_ids": result.inserted_ids
    }))
}

fn handle_embed_cache_stats(manager: &EmbeddingManager) -> Result<Value> {
    match manager.cache() {
        Some(cache) => {
            let stats = cache.stats();
            Ok(json!({
                "enabled": true,
                "stats": stats
            }))
        }
        None => Ok(json!({
            "enabled": false,
            "message": "Embedding cache is not enabled"
        })),
    }
}

fn handle_embed_cache_clear(manager: &EmbeddingManager) -> Result<Value> {
    match manager.cache() {
        Some(cache) => {
            let entries_before = cache.len();
            cache.clear();
            Ok(json!({
                "success": true,
                "entries_cleared": entries_before,
                "message": format!("Cleared {} cache entries", entries_before)
            }))
        }
        None => Ok(json!({
            "success": false,
            "message": "Embedding cache is not enabled"
        })),
    }
}
