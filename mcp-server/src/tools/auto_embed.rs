//! Auto-embedding tool handlers
//!
//! Provides MCP tools for configuring automatic embedding generation on collections.

use crate::adapter::IronBaseAdapter;
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use crate::jobs::{JobManager, JobType};
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

impl AutoEmbedEnableParams {
    /// Validate parameters
    pub fn validate(&self) -> Result<()> {
        // Collection name validation
        if self.collection.is_empty() {
            return Err(McpError::invalid_params("collection name cannot be empty"));
        }
        if self.collection.starts_with('_') && self.collection != "_system" {
            return Err(McpError::invalid_params(
                "collection names starting with '_' are reserved for system collections",
            ));
        }
        if self.collection.contains('\0') || self.collection.contains('/') {
            return Err(McpError::invalid_params(
                "collection name contains invalid characters (null or /)",
            ));
        }

        // Field name validation
        if self.source_field.is_empty() {
            return Err(McpError::invalid_params("source_field cannot be empty"));
        }
        if self.target_field.is_empty() {
            return Err(McpError::invalid_params("target_field cannot be empty"));
        }
        if self.source_field == self.target_field {
            return Err(McpError::invalid_params(
                "source_field and target_field cannot be the same",
            ));
        }

        // Provider validation
        if self.provider.is_empty() {
            return Err(McpError::invalid_params("provider cannot be empty"));
        }

        // Dimension validation
        if let Some(dim) = self.dimension {
            if dim == 0 {
                return Err(McpError::invalid_params("dimension must be greater than 0"));
            }
            if dim > 4096 {
                return Err(McpError::invalid_params(
                    "dimension cannot exceed 4096 (practical limit for most models)",
                ));
            }
        }

        // Chunking validation
        if let Some(ref chunking) = self.chunking {
            if chunking.chunk_size == 0 {
                return Err(McpError::invalid_params("chunk_size must be greater than 0"));
            }
            if chunking.overlap >= chunking.chunk_size {
                return Err(McpError::invalid_params(
                    "overlap must be less than chunk_size to prevent infinite loops",
                ));
            }
        }

        Ok(())
    }
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
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    match name {
        "auto_embed_enable" => handle_auto_embed_enable(params, adapter, embedding_manager),
        "auto_embed_disable" => handle_auto_embed_disable(params, adapter),
        "auto_embed_status" => handle_auto_embed_status(params, adapter),
        "auto_embed_backfill" => handle_auto_embed_backfill(params, adapter, embedding_manager, job_manager),
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

    // Validate input parameters
    p.validate()?;

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
    job_manager: &Option<Arc<JobManager>>,
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
    let _provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!("Provider '{}' not available", config.provider))
    })?;

    // Build filter: documents without target field (or where target is null)
    let mut query = p.filter.clone().unwrap_or(json!({}));
    if let Some(obj) = query.as_object_mut() {
        // Only process documents that don't have the embedding field
        obj.insert(
            config.target_field.clone(),
            json!({ "$exists": false })
        );
    }

    // For async backfill, use JobManager to run in background thread
    if p.r#async {
        let job_mgr = job_manager.as_ref().ok_or_else(|| {
            McpError::internal("Job manager not available for async operations")
        })?;

        // Check if shutdown is in progress
        if job_mgr.is_shutting_down() {
            return Err(McpError::internal(
                "Server is shutting down, cannot start new async jobs",
            ));
        }

        // Create job
        let job_type = JobType::EmbedBackfill {
            collection: p.collection.clone(),
            provider: config.provider.clone(),
        };
        let (job_id, _job) = job_mgr.create_job(job_type);

        // Clone necessary data for background thread
        let adapter_clone = adapter.clone();
        let manager_clone = manager.clone();
        let config_clone = config.clone();
        let collection = p.collection.clone();
        let batch_size = p.batch_size;
        let filter = p.filter.clone();
        let job_mgr_clone = job_mgr.clone();
        let job_id_clone = job_id.clone();
        let shutdown_flag = job_mgr.shutdown_flag();

        // Spawn background thread for processing
        let handle = std::thread::spawn(move || {
            run_backfill_job(
                &job_id_clone,
                &job_mgr_clone,
                &adapter_clone,
                &manager_clone,
                &config_clone,
                &collection,
                filter,
                batch_size,
                &shutdown_flag,
            );
        });

        // Register thread handle for graceful shutdown
        job_mgr.register_thread(job_id.clone(), handle);

        return Ok(json!({
            "success": true,
            "async": true,
            "job_id": job_id,
            "collection": p.collection,
            "message": "Backfill job started. Use embed_job_status to check progress."
        }));
    }

    // Get provider again for synchronous processing (after potential async branch)
    let provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!("Provider '{}' not available", config.provider))
    })?;

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

/// Background job execution for async backfill
#[allow(clippy::too_many_arguments)]
fn run_backfill_job(
    job_id: &str,
    job_manager: &JobManager,
    adapter: &IronBaseAdapter,
    embedding_manager: &EmbeddingManager,
    config: &AutoEmbeddingConfig,
    collection: &str,
    filter: Option<Value>,
    batch_size: usize,
    shutdown_flag: &std::sync::atomic::AtomicBool,
) {
    // Build filter: documents without target field
    let mut query = filter.unwrap_or(json!({}));
    if let Some(obj) = query.as_object_mut() {
        obj.insert(
            config.target_field.clone(),
            json!({ "$exists": false })
        );
    }

    // Count total documents to process
    let total_count = match adapter.count_documents(collection, query.clone()) {
        Ok(count) => count as usize,
        Err(e) => {
            job_manager.fail_job(job_id, format!("Failed to count documents: {}", e));
            return;
        }
    };

    if total_count == 0 {
        job_manager.complete_job(job_id, json!({
            "processed": 0,
            "errors": 0,
            "message": "No documents found that need embedding"
        }));
        return;
    }

    job_manager.update_progress(job_id, 0, Some(total_count), "Starting backfill...");

    // Get provider
    let provider = match embedding_manager.get_provider(&config.provider) {
        Some(p) => p,
        None => {
            job_manager.fail_job(job_id, format!("Provider '{}' not available", config.provider));
            return;
        }
    };

    let mut processed = 0;
    let mut errors = 0;
    let mut consecutive_empty_batches = 0;
    const MAX_EMPTY_BATCHES: usize = 3; // Safety limit to prevent infinite loop

    loop {
        // Check if shutdown was requested
        if shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!("Backfill job {} stopping due to shutdown", job_id);
            job_manager.update_progress(job_id, processed, Some(total_count), "Stopped (shutdown)");
            return;
        }

        // Check if job was cancelled
        if job_manager.is_cancelled(job_id) {
            job_manager.update_progress(job_id, processed, Some(total_count), "Cancelled");
            return;
        }

        // Fetch next batch - NO SKIP needed because the filter excludes already-processed docs
        // The query filter includes: target_field: {$exists: false}
        // So documents that have been processed (and now have target_field) are automatically excluded
        let find_options = crate::adapter::FindOptions {
            limit: Some(batch_size),
            skip: None, // Don't use skip - filter handles exclusion
            ..Default::default()
        };

        let result = match adapter.find(collection, query.clone(), find_options) {
            Ok(r) => r,
            Err(e) => {
                job_manager.fail_job(job_id, format!("Failed to fetch documents: {}", e));
                return;
            }
        };

        if result.documents.is_empty() {
            break;
        }

        // Extract texts from source field
        let texts: Vec<&str> = result.documents
            .iter()
            .filter_map(|doc| doc.get(&config.source_field).and_then(|v| v.as_str()))
            .collect();

        let batch_had_updates;

        if !texts.is_empty() {
            // Generate embeddings
            match provider.embed_batch(&texts) {
                Ok(embeddings) => {
                    // Update documents with embeddings
                    let docs_with_source: Vec<_> = result.documents
                        .iter()
                        .filter(|doc| doc.get(&config.source_field).and_then(|v| v.as_str()).is_some())
                        .collect();

                    let mut batch_processed = 0;
                    for (doc, embedding) in docs_with_source.iter().zip(embeddings.iter()) {
                        if let Some(id) = doc.get("_id") {
                            let filter = json!({ "_id": id });
                            let update = json!({
                                "$set": { &config.target_field: embedding }
                            });

                            match adapter.update_one(collection, filter, update) {
                                Ok(_) => {
                                    processed += 1;
                                    batch_processed += 1;
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to update document {:?}: {}", id, e);
                                    errors += 1;
                                }
                            }
                        }
                    }
                    batch_had_updates = batch_processed > 0;
                }
                Err(e) => {
                    tracing::warn!("Batch embedding failed: {}", e);
                    errors += texts.len();
                    batch_had_updates = false;
                }
            }
        } else {
            // All documents in batch are missing source field - they won't be processed
            // but they also won't be excluded by the filter, so we need to track this
            tracing::warn!(
                "Batch of {} documents has no valid source field '{}' - skipping",
                result.documents.len(),
                config.source_field
            );
            batch_had_updates = false;
        }

        // Safety: if we're not making progress, avoid infinite loop
        if !batch_had_updates {
            consecutive_empty_batches += 1;
            if consecutive_empty_batches >= MAX_EMPTY_BATCHES {
                tracing::warn!(
                    "Stopping backfill after {} consecutive batches with no updates",
                    MAX_EMPTY_BATCHES
                );
                break;
            }
        } else {
            consecutive_empty_batches = 0;
        }

        job_manager.update_progress(
            job_id,
            processed,
            Some(total_count),
            &format!("Processed {}/{}", processed, total_count),
        );

        // If we got fewer documents than batch_size, we're done
        if result.documents.len() < batch_size {
            break;
        }
    }

    // Complete the job
    job_manager.complete_job(job_id, json!({
        "processed": processed,
        "errors": errors,
        "total": total_count,
        "provider": config.provider
    }));
}
