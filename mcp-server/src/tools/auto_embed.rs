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

use super::defaults::{
    default_chunk_mode, default_chunk_overlap, default_chunk_size, effective_batch_size,
    is_allowed_system_collection, MAX_EMPTY_BATCHES,
};
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
    pub provider: Option<String>,
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
        if self.collection.starts_with('_') && !is_allowed_system_collection(&self.collection) {
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

        // Provider validation: if explicitly set, must be non-empty.
        // Missing/None is resolved to manager default at the handler site.
        if let Some(ref p) = self.provider {
            if p.is_empty() {
                return Err(McpError::invalid_params("provider cannot be empty"));
            }
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
                return Err(McpError::invalid_params(
                    "chunk_size must be greater than 0",
                ));
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
        "auto_embed_enable" => {
            handle_auto_embed_enable(params, adapter, embedding_manager, job_manager)
        }
        "auto_embed_disable" => handle_auto_embed_disable(params, adapter),
        "auto_embed_status" => handle_auto_embed_status(params, adapter),
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
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    let p: AutoEmbedEnableParams = AutoEmbedEnableParams::parse(params)?;

    // Validate input parameters
    p.validate()?;

    // Validate provider exists
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal("Embedding not available. Configure [embedding] section in config.toml.")
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

    // Check if provider is available
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

    // Get dimension from provider if not specified
    let dimension = p.dimension.unwrap_or_else(|| provider.dimension());

    // Build config — auto-save model name from provider
    let model_name = p
        .model
        .clone()
        .unwrap_or_else(|| provider.model_name().to_string());

    let chunking = p.chunking.map(|c| ChunkingConfig {
        mode: c.mode,
        chunk_size: c.chunk_size,
        overlap: c.overlap,
    });

    let config = AutoEmbeddingConfig {
        enabled: true,
        source_field: p.source_field.clone(),
        target_field: p.target_field.clone(),
        provider: provider_name,
        model: Some(model_name),
        dimension: Some(dimension),
        skip_if_exists: p.skip_if_exists,
        chunking,
        preprocessing_version: Some(provider.preprocessing_version().to_string()),
    };

    // Save to collection metadata
    adapter.set_auto_embedding_config(&p.collection, Some(config.clone()))?;

    // Count existing documents that need embedding (missing target_field)
    let backfill_query = json!({ &config.target_field: { "$exists": false } });
    let existing_count = adapter
        .count_documents(&p.collection, backfill_query)
        .unwrap_or(0);

    // Auto-start backfill if there are existing documents without embeddings
    let backfill_job_id = if existing_count > 0 {
        start_backfill_job(
            adapter,
            manager,
            &config,
            &p.collection,
            job_manager,
            false, // force=false: only docs missing target_field
        )
    } else {
        None
    };

    let mut response = json!({
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
        },
        "existing_documents": existing_count
    });

    if let Some(ref job_id) = backfill_job_id {
        response["backfill_job_id"] = json!(job_id);
        response["message"] =
            json!("Auto-embedding enabled. Backfill job started for existing documents.");
    }

    Ok(response)
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
        Some(config) => {
            let requires_reconfiguration = config.provider == "fasttext";
            let mut response = json!({
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
            });
            if requires_reconfiguration {
                response["requires_reconfiguration"] = json!(true);
                response["reconfiguration_reason"] = json!(
                    "Provider 'fasttext' has been removed. Call auto_embed_enable with a \
                     supported provider (ollama, vllm, openai) to re-enable this collection."
                );
            }
            Ok(response)
        }
        None => Ok(json!({
            "collection": p.collection,
            "enabled": false,
            "config": null
        })),
    }
}

/// Helper: start an async backfill job for a collection.
/// Returns Some(job_id) if job was started, None if job_manager unavailable or at capacity.
fn start_backfill_job(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Arc<EmbeddingManager>,
    config: &AutoEmbeddingConfig,
    collection: &str,
    job_manager: &Option<Arc<JobManager>>,
    force: bool,
) -> Option<String> {
    let job_mgr = job_manager.as_ref()?;

    if job_mgr.is_shutting_down() {
        tracing::warn!("Cannot start backfill: server is shutting down");
        return None;
    }

    if !job_mgr.can_start_job() {
        tracing::warn!(
            "Cannot start backfill for '{}': max concurrent jobs reached",
            collection
        );
        return None;
    }

    let job_type = JobType::EmbedBackfill {
        collection: collection.to_string(),
        provider: config.provider.clone(),
    };
    let (job_id, _job) = job_mgr.create_job(job_type);

    let adapter_clone = adapter.clone();
    let manager_clone = embedding_manager.clone();
    let config_clone = config.clone();
    let collection_owned = collection.to_string();
    let batch_size = effective_batch_size();
    let job_mgr_clone = job_mgr.clone();
    let job_id_clone = job_id.clone();
    let shutdown_flag = job_mgr.shutdown_flag();

    let handle = std::thread::spawn(move || {
        run_backfill_job(
            &job_id_clone,
            &job_mgr_clone,
            &adapter_clone,
            &manager_clone,
            &config_clone,
            &collection_owned,
            batch_size,
            force,
            &shutdown_flag,
        );
    });

    job_mgr.register_thread(job_id.clone(), handle);

    tracing::info!(
        "Backfill job {} started for '{}' (force={})",
        job_id,
        collection,
        force
    );

    Some(job_id)
}

/// Handle dimension change: drop old HNSW index, $unset old vectors, create new index.
///
/// Crash-safe ordering:
/// 1. Drop old HNSW → if crash: no vector index, collection scan works
/// 2. $unset old vectors → if crash: no vector data, re-embed will fix
/// 3. Create new HNSW → if crash: empty index, re-embed will populate
fn handle_dimension_change(
    adapter: &Arc<IronBaseAdapter>,
    collection: &str,
    target_field: &str,
    new_dimension: usize,
) -> std::result::Result<(), String> {
    // 1. Find and drop existing vector indexes on target_field
    let indexes = adapter
        .list_vector_indexes(collection)
        .map_err(|e| format!("Failed to list vector indexes: {}", e))?;

    for idx in &indexes {
        let field = idx.get("field").and_then(|v| v.as_str()).unwrap_or("");
        if field == target_field {
            let name = idx.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() {
                tracing::info!("Dropping old vector index '{}' on '{}'", name, collection);
                adapter
                    .drop_vector_index(collection, name)
                    .map_err(|e| format!("Failed to drop vector index '{}': {}", name, e))?;
            }
        }
    }

    // 2. $unset old vectors (wrong dimension) — batch update to avoid OOM
    tracing::info!(
        "Clearing old embedding vectors from '{}' field '{}' (wrong dimension)",
        collection,
        target_field
    );
    let update = json!({ "$unset": { target_field: "" } });
    let filter = json!({ target_field: { "$exists": true } });
    if let Err(e) = adapter.update_many(collection, filter, update) {
        tracing::warn!(
            "Failed to $unset old vectors for '{}': {} (re-embed will overwrite anyway)",
            collection,
            e
        );
    }

    // 3. Create new HNSW index with correct dimension
    tracing::info!(
        "Creating new vector index on '{}' field '{}' with dim={}",
        collection,
        target_field,
        new_dimension
    );
    adapter
        .create_vector_index(
            collection,
            target_field,
            new_dimension,
            "cosine", // default metric
            100_000,  // max_vectors
            16,       // m
            200,      // ef_construction
            50,       // ef_search
        )
        .map_err(|e| format!("Failed to create vector index: {}", e))?;

    Ok(())
}

/// Background job execution for async backfill.
///
/// If `force` is true, re-embeds ALL documents (used for model change re-embedding).
/// If `force` is false, only embeds documents missing the target_field.
#[allow(clippy::too_many_arguments)]
fn run_backfill_job(
    job_id: &str,
    job_manager: &JobManager,
    adapter: &IronBaseAdapter,
    embedding_manager: &EmbeddingManager,
    config: &AutoEmbeddingConfig,
    collection: &str,
    batch_size: usize,
    force: bool,
    shutdown_flag: &std::sync::atomic::AtomicBool,
) {
    // Build filter: if not force, only documents without target field
    let mut query = json!({});
    if !force {
        if let Some(obj) = query.as_object_mut() {
            obj.insert(config.target_field.clone(), json!({ "$exists": false }));
        }
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
        job_manager.complete_job(
            job_id,
            json!({
                "processed": 0,
                "errors": 0,
                "message": "No documents found that need embedding"
            }),
        );
        return;
    }

    job_manager.update_progress(job_id, 0, Some(total_count), "Starting backfill...");

    // Get provider
    let provider = match embedding_manager.get_provider(&config.provider) {
        Some(p) => p,
        None => {
            job_manager.fail_job(
                job_id,
                format!("Provider '{}' not available", config.provider),
            );
            return;
        }
    };

    let mut processed = 0;
    let mut errors = 0;
    let mut consecutive_empty_batches = 0;
    // For force mode: use skip-based pagination since all docs match the empty filter.
    // For non-force mode: no skip needed since processed docs gain target_field and are excluded.
    let mut skip_offset: usize = 0;

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

        let find_options = crate::adapter::FindOptions {
            limit: Some(batch_size),
            skip: if force { Some(skip_offset) } else { None },
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
        let texts: Vec<&str> = result
            .documents
            .iter()
            .filter_map(|doc| doc.get(&config.source_field).and_then(|v| v.as_str()))
            .collect();

        let batch_had_updates = if !texts.is_empty() {
            // Generate embeddings — try batch first, fallback to individual on failure
            let docs_with_source: Vec<_> = result
                .documents
                .iter()
                .filter(|doc| {
                    doc.get(&config.source_field)
                        .and_then(|v| v.as_str())
                        .is_some()
                })
                .collect();

            let embeddings_result = provider.embed_batch(&texts);

            let mut batch_processed = 0;

            // Store embedding into document via update_one
            let store_one = |id: &Value, embedding: &[f32]| -> bool {
                let filter = json!({ "_id": id });
                let update = json!({
                    "$set": { &config.target_field: embedding }
                });
                match adapter.update_one(collection, filter, update) {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::warn!("Failed to update document {:?}: {}", id, e);
                        false
                    }
                }
            };

            match embeddings_result {
                Ok(embeddings) => {
                    for (doc, embedding) in docs_with_source.iter().zip(embeddings.iter()) {
                        if let Some(id) = doc.get("_id") {
                            if store_one(id, embedding) {
                                processed += 1;
                                batch_processed += 1;
                            } else {
                                errors += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    // Batch failed (e.g., NaN from problematic texts) — fallback to individual
                    tracing::warn!(
                        "Batch embedding failed ({}), falling back to individual embedding for {} texts",
                        e, texts.len()
                    );

                    for (doc, text) in docs_with_source.iter().zip(texts.iter()) {
                        if let Some(id) = doc.get("_id") {
                            match provider.embed(text) {
                                Ok(embedding) => {
                                    if store_one(id, &embedding) {
                                        processed += 1;
                                        batch_processed += 1;
                                    } else {
                                        errors += 1;
                                    }
                                }
                                Err(_) => {
                                    // Individual text also fails (NaN) — skip it
                                    tracing::debug!(
                                        "Skipping document {:?}: embedding produces NaN (text len={})",
                                        id, text.len()
                                    );
                                    errors += 1;
                                }
                            }
                        }
                    }
                }
            }
            batch_processed > 0
        } else {
            // All documents in batch are missing source field - they won't be processed
            // but they also won't be excluded by the filter, so we need to track this
            tracing::warn!(
                "Batch of {} documents has no valid source field '{}' - skipping",
                result.documents.len(),
                config.source_field
            );
            false
        };

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

        // Advance skip offset for force mode
        if force {
            skip_offset += result.documents.len();
        }

        // If we got fewer documents than batch_size, we're done
        if result.documents.len() < batch_size {
            break;
        }
    }

    // Complete the job
    job_manager.complete_job(
        job_id,
        json!({
            "processed": processed,
            "errors": errors,
            "total": total_count,
            "provider": config.provider
        }),
    );
}

// ============================================================================
// Startup Hook: Model Change Detection
// ============================================================================

/// Check all collections for embedding model/preprocessing changes and start re-embedding if needed.
///
/// Called at server startup. For each collection with auto-embedding enabled:
/// - If both `config.model` and `config.preprocessing_version` are empty (legacy config):
///   saves current values, no re-embed
/// - If model or preprocessing version differs: starts force re-embed
/// - If `force_reembed` is true: starts force re-embed regardless
/// - If everything matches: no action
pub fn check_model_changes_and_reembed(
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
    job_manager: &Option<Arc<JobManager>>,
    force_reembed: bool,
) {
    let manager = match embedding_manager.as_ref() {
        Some(m) => m,
        None => return, // No embedding manager → nothing to check
    };

    let collections = adapter.list_collections();

    for collection in &collections {
        // Skip system collections
        if collection.starts_with('_') {
            continue;
        }

        let config = match adapter.get_auto_embedding_config(collection) {
            Ok(Some(c)) if c.enabled => c,
            _ => continue,
        };

        // FastText support was removed. Collections still configured with provider="fasttext"
        // must be reconfigured manually via auto_embed_enable with a current provider
        // (ollama, vllm, openai) — re-embedding will be triggered automatically afterwards.
        if config.provider == "fasttext" {
            tracing::error!(
                "Collection '{}': stored auto-embed config uses provider='fasttext', which has been removed. \
                 Reconfigure via auto_embed_enable with a current provider (ollama, vllm, openai). \
                 This collection will not auto-embed until reconfigured.",
                collection
            );
            continue;
        }

        let provider = match manager.get_provider(&config.provider) {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "Collection '{}': auto-embed provider '{}' not available (configured provider: {}). \
                     To fix: either reconfigure with auto_embed_enable using the current provider, \
                     or add the '{}' provider to [embedding] config.",
                    collection,
                    config.provider,
                    manager.default_provider_name(),
                    config.provider
                );
                continue;
            }
        };

        let current_model = provider.model_name().to_string();
        let saved_model = config.model.as_deref().unwrap_or("");
        let current_preproc = provider.preprocessing_version().to_string();
        let saved_preproc = config.preprocessing_version.as_deref().unwrap_or("");
        let current_dim = provider.dimension();

        // Check dimension change (e.g., 300 → BGE-M3 1024)
        let saved_dim = adapter
            .list_vector_indexes(collection)
            .ok()
            .and_then(|indexes| {
                indexes.iter().find_map(|idx| {
                    let field = idx.get("field")?.as_str()?;
                    if field == config.target_field {
                        idx.get("dim")?.as_u64().map(|d| d as usize)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0);

        let dimension_changed = saved_dim > 0 && saved_dim != current_dim;

        if dimension_changed {
            tracing::info!(
                "Dimension change detected for '{}': {} → {} on field '{}', rebuilding HNSW index",
                collection,
                saved_dim,
                current_dim,
                config.target_field
            );
            if let Err(e) =
                handle_dimension_change(adapter, collection, &config.target_field, current_dim)
            {
                tracing::error!(
                    "Failed to handle dimension change for '{}': {}",
                    collection,
                    e
                );
                continue;
            }
        }

        if saved_model.is_empty()
            && saved_preproc.is_empty()
            && !force_reembed
            && !dimension_changed
        {
            // Legacy config without model/preprocessing info — save current, don't re-embed
            tracing::info!(
                "Collection '{}': saving model '{}' and preprocessing '{}' to config (first time)",
                collection,
                current_model,
                current_preproc
            );
            let mut updated_config = config.clone();
            updated_config.model = Some(current_model);
            updated_config.preprocessing_version = Some(current_preproc);
            if let Err(e) = adapter.set_auto_embedding_config(collection, Some(updated_config)) {
                tracing::warn!("Failed to update config for '{}': {}", collection, e);
            }
        } else if saved_model != current_model
            || saved_preproc != current_preproc
            || force_reembed
            || dimension_changed
        {
            // Something changed or force requested — start re-embed
            let reason = if force_reembed {
                "force flag".to_string()
            } else if saved_model != current_model && saved_preproc != current_preproc {
                format!(
                    "model '{}' → '{}' and preprocessing '{}' → '{}'",
                    saved_model, current_model, saved_preproc, current_preproc
                )
            } else if saved_model != current_model {
                format!("model '{}' → '{}'", saved_model, current_model)
            } else {
                format!("preprocessing '{}' → '{}'", saved_preproc, current_preproc)
            };

            tracing::info!(
                "Re-embed triggered for '{}': {}, starting re-embed",
                collection,
                reason
            );

            // Update config with current values
            let mut updated_config = config.clone();
            updated_config.model = Some(current_model);
            updated_config.preprocessing_version = Some(current_preproc);
            if let Err(e) =
                adapter.set_auto_embedding_config(collection, Some(updated_config.clone()))
            {
                tracing::warn!("Failed to update config for '{}': {}", collection, e);
                continue;
            }

            // Start force backfill (re-embed ALL documents)
            start_backfill_job(
                adapter,
                manager,
                &updated_config,
                collection,
                job_manager,
                true,
            );
        }
    }
}
