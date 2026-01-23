//! Auto-embedding tool handlers
//!
//! Provides MCP tools for configuring automatic embedding generation on collections.

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use ironbase_core::storage::{AutoEmbeddingConfig, ChunkingConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::params::ParseParams;

// ============================================================================
// Parameter Structs
// ============================================================================

/// Parameters for `auto_embed_enable` tool
#[derive(Debug, Deserialize)]
pub struct AutoEmbedEnableParams {
    pub collection: String,
    pub source_field: String,
    pub target_field: String,
    pub provider: String,
    pub model: Option<String>,
    pub dimension: Option<usize>,
    #[serde(default)]
    pub skip_if_exists: bool,
    pub chunking: Option<ChunkingParams>,
}

/// Chunking parameters (subset of ChunkingConfig for MCP)
#[derive(Debug, Deserialize)]
pub struct ChunkingParams {
    #[serde(default = "default_chunk_mode")]
    pub mode: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_chunk_overlap")]
    pub overlap: usize,
}

fn default_chunk_mode() -> String {
    "auto".to_string()
}

fn default_chunk_size() -> usize {
    1000
}

fn default_chunk_overlap() -> usize {
    100
}

/// Parameters for `auto_embed_disable` tool
#[derive(Debug, Deserialize)]
pub struct AutoEmbedDisableParams {
    pub collection: String,
}

/// Parameters for `auto_embed_status` tool
#[derive(Debug, Deserialize)]
pub struct AutoEmbedStatusParams {
    pub collection: String,
}

/// Parameters for `auto_embed_backfill` tool
#[derive(Debug, Deserialize)]
pub struct AutoEmbedBackfillParams {
    pub collection: String,
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_async")]
    pub r#async: bool,
}

fn default_batch_size() -> usize {
    100
}

fn default_async() -> bool {
    true
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch auto-embedding tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "auto_embed_enable" => handle_auto_embed_enable(params, adapter, embedding_manager),
        "auto_embed_disable" => handle_auto_embed_disable(params, adapter),
        "auto_embed_status" => handle_auto_embed_status(params, adapter),
        "auto_embed_backfill" => handle_auto_embed_backfill(params, adapter, embedding_manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown auto-embed tool: {}",
            name
        ))),
    }
}

// ============================================================================
// Tool Handlers
// ============================================================================

fn handle_auto_embed_enable(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: AutoEmbedEnableParams = AutoEmbedEnableParams::parse(params)?;

    // Validate provider exists
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
        )
    })?;

    // Check if provider is available
    if manager.get_provider(&p.provider).is_none() {
        let available: Vec<_> = manager.list_models().iter().map(|m| m.provider.clone()).collect();
        return Err(McpError::invalid_params(format!(
            "Provider '{}' not found. Available: {:?}",
            p.provider, available
        )));
    }

    // Get dimension from provider if not specified
    let dimension = p.dimension.or_else(|| {
        manager
            .get_provider(&p.provider)
            .map(|prov| prov.dimension())
    });

    // Build config
    let chunking = p.chunking.map(|c| ChunkingConfig {
        mode: c.mode,
        chunk_size: c.chunk_size,
        overlap: c.overlap,
    });

    let config = AutoEmbeddingConfig {
        enabled: true,
        source_field: p.source_field.clone(),
        target_field: p.target_field.clone(),
        provider: p.provider.clone(),
        model: p.model.clone(),
        dimension,
        skip_if_exists: p.skip_if_exists,
        chunking,
    };

    // Save to collection metadata
    adapter.set_auto_embedding_config(&p.collection, Some(config.clone()))?;

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "config": {
            "enabled": config.enabled,
            "source_field": config.source_field,
            "target_field": config.target_field,
            "provider": config.provider,
            "model": config.model,
            "dimension": config.dimension,
            "skip_if_exists": config.skip_if_exists,
            "chunking": config.chunking.map(|c| json!({
                "mode": c.mode,
                "chunk_size": c.chunk_size,
                "overlap": c.overlap
            }))
        }
    }))
}

fn handle_auto_embed_disable(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: AutoEmbedDisableParams = AutoEmbedDisableParams::parse(params)?;

    // Set config to None (disables auto-embedding)
    adapter.set_auto_embedding_config(&p.collection, None)?;

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "message": "Auto-embedding disabled"
    }))
}

fn handle_auto_embed_status(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: AutoEmbedStatusParams = AutoEmbedStatusParams::parse(params)?;

    let config = adapter.get_auto_embedding_config(&p.collection)?;

    match config {
        Some(config) => Ok(json!({
            "collection": p.collection,
            "enabled": config.enabled,
            "config": {
                "source_field": config.source_field,
                "target_field": config.target_field,
                "provider": config.provider,
                "model": config.model,
                "dimension": config.dimension,
                "skip_if_exists": config.skip_if_exists,
                "chunking": config.chunking.map(|c| json!({
                    "mode": c.mode,
                    "chunk_size": c.chunk_size,
                    "overlap": c.overlap
                }))
            }
        })),
        None => Ok(json!({
            "collection": p.collection,
            "enabled": false,
            "config": null
        })),
    }
}

fn handle_auto_embed_backfill(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    let p: AutoEmbedBackfillParams = AutoEmbedBackfillParams::parse(params)?;

    // Get auto-embedding config
    let config = adapter.get_auto_embedding_config(&p.collection)?
        .ok_or_else(|| McpError::invalid_params(format!(
            "Auto-embedding not configured for collection '{}'. Use auto_embed_enable first.",
            p.collection
        )))?;

    if !config.enabled {
        return Err(McpError::invalid_params(format!(
            "Auto-embedding is disabled for collection '{}'. Use auto_embed_enable first.",
            p.collection
        )));
    }

    // Validate embedding manager
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal("Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.")
    })?;

    // Check if provider is available
    let provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!("Provider '{}' not available", config.provider))
    })?;

    // Build filter: documents without target field (or where target is null)
    let mut query = p.filter.unwrap_or(json!({}));
    if let Some(obj) = query.as_object_mut() {
        // Only process documents that don't have the embedding field
        obj.insert(
            config.target_field.clone(),
            json!({ "$exists": false })
        );
    }

    // For async backfill, we'd return a job ID and process in background
    // For now, we'll do synchronous processing with progress reporting
    if p.r#async {
        // TODO: Implement async job manager (Phase 4)
        // For now, return a placeholder indicating async is not yet implemented
        return Ok(json!({
            "success": false,
            "message": "Async backfill not yet implemented. Use async=false for synchronous processing.",
            "collection": p.collection
        }));
    }

    // Synchronous processing
    let filter = crate::adapter::FindOptions {
        limit: Some(p.batch_size * 10), // Process up to 10 batches
        ..Default::default()
    };

    let result = adapter.find(&p.collection, query.clone(), filter)?;
    let total_docs = result.documents.len();

    if total_docs == 0 {
        return Ok(json!({
            "success": true,
            "collection": p.collection,
            "processed": 0,
            "message": "No documents found that need embedding"
        }));
    }

    let mut processed = 0;
    let mut errors = 0;

    // Process in batches
    for batch in result.documents.chunks(p.batch_size) {
        // Extract texts from source field
        let texts: Vec<&str> = batch
            .iter()
            .filter_map(|doc| doc.get(&config.source_field).and_then(|v| v.as_str()))
            .collect();

        if texts.is_empty() {
            continue;
        }

        // Generate embeddings
        let embeddings = match provider.embed_batch(&texts) {
            Ok(embs) => embs,
            Err(e) => {
                tracing::warn!("Batch embedding failed: {}", e);
                errors += texts.len();
                continue;
            }
        };

        // Update documents with embeddings
        for (doc, embedding) in batch.iter().zip(embeddings.iter()) {
            if let Some(id) = doc.get("_id") {
                let filter = json!({ "_id": id });
                let update = json!({
                    "$set": { &config.target_field: embedding }
                });

                match adapter.update_one(&p.collection, filter, update) {
                    Ok(_) => processed += 1,
                    Err(e) => {
                        tracing::warn!("Failed to update document: {}", e);
                        errors += 1;
                    }
                }
            }
        }
    }

    Ok(json!({
        "success": true,
        "collection": p.collection,
        "total_found": total_docs,
        "processed": processed,
        "errors": errors,
        "provider": config.provider,
        "dimension": provider.dimension()
    }))
}
