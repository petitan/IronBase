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

    // Visibility: warn if this looks like a chunk-imported (RAG) collection.
    // Generic auto-embed embeds source_field verbatim — without the section
    // breadcrumb that rag_document_import adds — so re-embedded vectors would
    // differ from import-time vectors. Surface this instead of degrading silently.
    let chunk_imported = adapter
        .find_one(
            &p.collection,
            json!({ "section_path": { "$exists": true } }),
        )
        .map(|opt| opt.is_some())
        .unwrap_or(false);
    let verbatim_warning = if chunk_imported {
        let msg = format!(
            "Collection '{}' contains chunk documents (section_path present). Auto-embed embeds \
             '{}' verbatim, without the section breadcrumb that rag_document_import adds — these \
             vectors will differ from import-time vectors.",
            p.collection, config.source_field
        );
        tracing::warn!("{}", msg);
        Some(msg)
    } else {
        None
    };

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

    if let Some(msg) = verbatim_warning {
        response["warning"] = json!(msg);
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

    // The new index keeps the old one's metric and HNSW parameters; only the
    // dimension changes (defaults only if no index existed on the field)
    let old = indexes
        .iter()
        .find(|idx| idx.get("field").and_then(|v| v.as_str()) == Some(target_field));
    let old_param = |key: &str, default: usize| -> usize {
        old.and_then(|idx| {
            idx.get("hnsw_params")
                .and_then(|p| p.get(key))
                .or_else(|| idx.get(key))
        })
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
    };
    let metric = old
        .and_then(|idx| idx.get("metric").and_then(|v| v.as_str()))
        .unwrap_or("cosine")
        .to_string();
    let max_vectors = old_param("max_vectors", 100_000);
    let m = old_param("m", 16);
    let ef_construction = old_param("ef_construction", 200);
    let ef_search = old_param("ef_search", 50);

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
        "Creating new vector index on '{}' field '{}' with dim={} (metric={}, m={}, ef_construction={})",
        collection,
        target_field,
        new_dimension,
        metric,
        m,
        ef_construction
    );
    adapter
        .create_vector_index(
            collection,
            target_field,
            new_dimension,
            &metric,
            max_vectors,
            m,
            ef_construction,
            ef_search,
        )
        .map_err(|e| format!("Failed to create vector index: {}", e))?;

    Ok(())
}

/// Background job execution for async backfill.
///
/// If `force` is true, re-embeds ALL documents (used for model change re-embedding).
/// If `force` is false, only embeds documents missing the target_field.
///
/// A forced run saves `config.model` / `config.preprocessing_version` to the
/// collection's auto-embed config only after it reached the end of the
/// collection; documents it could not embed lose their old vector. Until then the stored values keep describing the old vectors,
/// so a run that is interrupted, fails or never starts is detected and restarted
/// by the next startup check instead of leaving a mixed embedding space.
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
        let mut summary = json!({
            "processed": 0,
            "errors": 0,
            "message": "No documents found that need embedding"
        });
        if force {
            // Nothing to re-embed: the (empty) space is already the new one
            summary["embedding_space_committed"] =
                json!(commit_embedding_space(adapter, collection, config));
        }
        job_manager.complete_job(job_id, summary);
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
    // Set only when the loop reaches the end of the collection (not on the
    // no-progress safety stop)
    let mut traversed_all = false;
    // Forced run: documents left with an old-model vector (embedding failed)
    let mut failed_ids: Vec<Value> = Vec::new();
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
            traversed_all = true;
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
                    // A short response leaves the remaining documents unembedded
                    for doc in docs_with_source.iter().skip(embeddings.len()) {
                        errors += 1;
                        if let (true, Some(id)) = (force, doc.get("_id")) {
                            failed_ids.push(id.clone());
                        }
                    }
                    for (doc, embedding) in docs_with_source.iter().zip(embeddings.iter()) {
                        if let Some(id) = doc.get("_id") {
                            if store_one(id, embedding) {
                                processed += 1;
                                batch_processed += 1;
                            } else {
                                errors += 1;
                                if force {
                                    failed_ids.push(id.clone());
                                }
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
                        // One embedding call per document: stop on cancel
                        // instead of finishing the batch (M3)
                        if job_manager.is_cancelled(job_id) {
                            break;
                        }
                        if let Some(id) = doc.get("_id") {
                            match provider.embed(text) {
                                Ok(embedding) => {
                                    if store_one(id, &embedding) {
                                        processed += 1;
                                        batch_processed += 1;
                                    } else {
                                        errors += 1;
                                        if force {
                                            failed_ids.push(id.clone());
                                        }
                                    }
                                }
                                Err(_) => {
                                    // Individual text also fails (NaN) — skip it
                                    tracing::debug!(
                                        "Skipping document {:?}: embedding produces NaN (text len={})",
                                        id, text.len()
                                    );
                                    errors += 1;
                                    if force {
                                        failed_ids.push(id.clone());
                                    }
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

        // Safety: if we're not making progress, avoid infinite loop. A forced run
        // pages with skip and always advances, so there a batch without source
        // texts is not a stall — only a batch whose embeddings all failed is.
        let stalled = if force {
            !texts.is_empty() && !batch_had_updates
        } else {
            !batch_had_updates
        };
        if stalled {
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
            traversed_all = true;
            break;
        }
    }

    let mut summary = json!({
        "processed": processed,
        "errors": errors,
        "total": total_count,
        "provider": config.provider
    });

    if force {
        // A document that cannot be embedded with the new model must not keep
        // its old-model vector (it would be ranked in the wrong space); it is
        // left without an embedding instead. Only on a full pass: a stopped run
        // (e.g. provider outage) keeps everything and is restarted.
        let committed = traversed_all
            && remove_stale_vectors(adapter, collection, &config.target_field, &failed_ids)
            && commit_embedding_space(adapter, collection, config);
        if !committed {
            tracing::warn!(
                "Re-embed of '{}' incomplete ({} errors, reached end: {}); the stored model \
                 is kept and the re-embed restarts on the next startup",
                collection,
                errors,
                traversed_all
            );
        }
        summary["embedding_space_committed"] = json!(committed);
        summary["stale_vectors_removed"] = json!(if traversed_all { failed_ids.len() } else { 0 });
    }

    // Complete the job
    job_manager.complete_job(job_id, summary);
}

/// Unset the target field of the documents a forced re-embed could not embed.
/// Returns false if any of them still holds its old vector.
fn remove_stale_vectors(
    adapter: &IronBaseAdapter,
    collection: &str,
    target_field: &str,
    ids: &[Value],
) -> bool {
    let mut all_removed = true;
    for id in ids {
        let update = json!({ "$unset": { target_field: "" } });
        if let Err(e) = adapter.update_one(collection, json!({ "_id": id }), update) {
            tracing::warn!("Failed to remove stale vector of {:?}: {}", id, e);
            all_removed = false;
        }
    }
    if !ids.is_empty() {
        tracing::warn!(
            "Re-embed of '{}': {} documents could not be embedded; their old vectors were removed",
            collection,
            ids.len()
        );
    }
    all_removed
}

/// Save the model and preprocessing version a forced re-embed produced into the
/// collection's auto-embed config. Skipped when the config was disabled or
/// pointed elsewhere while the job ran.
fn commit_embedding_space(
    adapter: &IronBaseAdapter,
    collection: &str,
    target: &AutoEmbeddingConfig,
) -> bool {
    let mut current = match adapter.get_auto_embedding_config(collection) {
        Ok(Some(c))
            if c.enabled
                && c.provider == target.provider
                && c.target_field == target.target_field =>
        {
            c
        }
        _ => {
            tracing::warn!(
                "Auto-embed config of '{}' changed during re-embed; not recording model '{}'",
                collection,
                target.model.as_deref().unwrap_or("")
            );
            return false;
        }
    };
    current.model = target.model.clone();
    current.preprocessing_version = target.preprocessing_version.clone();
    match adapter.set_auto_embedding_config(collection, Some(current)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("Failed to update config for '{}': {}", collection, e);
            false
        }
    }
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
///
/// The new model/preprocessing are saved by the re-embed job when it finishes
/// cleanly, so an interrupted re-embed is restarted here on the next startup.
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

            // The stored model/preprocessing describe the existing vectors, so
            // they are NOT updated here: the job records the new values once all
            // documents are re-embedded (see run_backfill_job)
            let mut target_config = config.clone();
            target_config.model = Some(current_model);
            target_config.preprocessing_version = Some(current_preproc);

            // Start force backfill (re-embed ALL documents)
            if start_backfill_job(
                adapter,
                manager,
                &target_config,
                collection,
                job_manager,
                true,
            )
            .is_none()
            {
                tracing::warn!(
                    "Re-embed for '{}' could not be started; it will be retried on the next startup",
                    collection
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{EmbeddingError, EmbeddingProvider, EmbeddingResult};
    use crate::jobs::JobStatus;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    /// Embeds a text as [length, 2.0] under model "new"; fails on "bad"
    struct NewModelProvider;

    impl EmbeddingProvider for NewModelProvider {
        fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
            if text == "bad" {
                return Err(EmbeddingError::ApiError("boom".to_string()));
            }
            Ok(vec![text.chars().count() as f32, 2.0])
        }
        fn dimension(&self) -> usize {
            2
        }
        fn model_name(&self) -> &str {
            "new"
        }
        fn provider_name(&self) -> &str {
            "mock"
        }
    }

    /// Collection "kb" whose vectors (all [0, 0]) were made by model "old"
    fn setup(contents: &[Option<&str>]) -> (Arc<IronBaseAdapter>, Arc<EmbeddingManager>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let adapter = Arc::new(
            IronBaseAdapter::new(tmp.path().join("t.mlite").to_string_lossy().to_string()).unwrap(),
        );
        for (i, content) in contents.iter().enumerate() {
            let mut doc = json!({"_id": i as i64, "embedding": [0.0, 0.0]});
            if let Some(c) = content {
                doc["content"] = json!(c);
            }
            adapter.insert_one("kb", doc).unwrap();
        }
        adapter
            .set_auto_embedding_config(
                "kb",
                Some(AutoEmbeddingConfig {
                    enabled: true,
                    source_field: "content".to_string(),
                    target_field: "embedding".to_string(),
                    provider: "mock".to_string(),
                    model: Some("old".to_string()),
                    dimension: Some(2),
                    preprocessing_version: Some("default".to_string()),
                    ..Default::default()
                }),
            )
            .unwrap();
        let mut manager = EmbeddingManager::new();
        manager.register_provider("mock", Arc::new(NewModelProvider));
        (adapter, Arc::new(manager), tmp)
    }

    fn stored_model(adapter: &IronBaseAdapter) -> Option<String> {
        adapter
            .get_auto_embedding_config("kb")
            .unwrap()
            .unwrap()
            .model
    }

    /// Run a forced re-embed to model "new" synchronously; returns the job result
    fn run_forced(
        adapter: &IronBaseAdapter,
        manager: &EmbeddingManager,
        batch_size: usize,
        cancel_first: bool,
    ) -> Option<Value> {
        let job_manager = JobManager::new();
        let (job_id, _) = job_manager.create_job(JobType::EmbedBackfill {
            collection: "kb".to_string(),
            provider: "mock".to_string(),
        });
        if cancel_first {
            job_manager.cancel_job(&job_id);
        }
        let mut target = adapter.get_auto_embedding_config("kb").unwrap().unwrap();
        target.model = Some("new".to_string());
        run_backfill_job(
            &job_id,
            &job_manager,
            adapter,
            manager,
            &target,
            "kb",
            batch_size,
            true,
            &AtomicBool::new(false),
        );
        match job_manager.get_job_info(&job_id).unwrap().status {
            JobStatus::Completed { result } => Some(result),
            _ => None,
        }
    }

    /// Hybrid review F4: the startup check must not record the new model before
    /// the vectors are re-embedded — here the job cannot even start
    #[test]
    fn startup_check_keeps_old_model_until_reembedded() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de")]);
        check_model_changes_and_reembed(&adapter, &Some(manager), &None, false);

        assert_eq!(stored_model(&adapter).as_deref(), Some("old"));
    }

    #[test]
    fn forced_reembed_records_model_after_all_documents() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de"), Some("f")]);
        let result = run_forced(&adapter, &manager, 2, false).unwrap();

        assert_eq!(result["embedding_space_committed"], json!(true));
        assert_eq!(stored_model(&adapter).as_deref(), Some("new"));
        for doc in adapter
            .find("kb", json!({}), Default::default())
            .unwrap()
            .documents
        {
            assert_eq!(doc["embedding"][1], json!(2.0), "{}", doc);
        }
    }

    /// A document the new model cannot embed loses its old-model vector instead
    /// of staying in the index (and does not block recording the new model)
    #[test]
    fn forced_reembed_removes_vectors_it_cannot_replace() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("bad"), Some("f")]);
        let result = run_forced(&adapter, &manager, 10, false).unwrap();

        assert_eq!(result["errors"], json!(1));
        assert_eq!(result["stale_vectors_removed"], json!(1));
        assert_eq!(result["embedding_space_committed"], json!(true));
        assert_eq!(stored_model(&adapter).as_deref(), Some("new"));
        let bad = adapter.find_one("kb", json!({"_id": 1})).unwrap().unwrap();
        assert!(bad.get("embedding").is_none(), "{}", bad);
    }

    /// Every batch failing (provider outage) stops the run early: nothing is
    /// removed and the old model stays recorded, so the next startup retries
    #[test]
    fn forced_reembed_stopped_by_failures_keeps_old_model() {
        let mut contents = vec![Some("bad"); MAX_EMPTY_BATCHES + 2];
        contents.push(Some("abc"));
        let (adapter, manager, _t) = setup(&contents);
        let result = run_forced(&adapter, &manager, 1, false).unwrap();

        assert_eq!(result["embedding_space_committed"], json!(false));
        assert_eq!(result["stale_vectors_removed"], json!(0));
        assert_eq!(stored_model(&adapter).as_deref(), Some("old"));
        let still_embedded = adapter
            .count_documents("kb", json!({"embedding": {"$exists": true}}))
            .unwrap();
        assert_eq!(still_embedded as usize, MAX_EMPTY_BATCHES + 3);
    }

    #[test]
    fn cancelled_forced_reembed_keeps_old_model() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de")]);
        assert!(run_forced(&adapter, &manager, 10, true).is_none());

        assert_eq!(stored_model(&adapter).as_deref(), Some("old"));
    }

    /// A forced run pages with skip: batches of documents without source text
    /// must not trip the no-progress stop before the end of the collection
    #[test]
    fn forced_reembed_passes_documents_without_source() {
        let mut contents = vec![None; 12];
        contents.push(Some("abc"));
        let (adapter, manager, _t) = setup(&contents);
        let result = run_forced(&adapter, &manager, 1, false).unwrap();

        assert_eq!(result["processed"], json!(1));
        assert_eq!(result["embedding_space_committed"], json!(true));
        assert_eq!(stored_model(&adapter).as_deref(), Some("new"));
    }

    #[test]
    fn forced_reembed_of_empty_collection_records_model() {
        let (adapter, manager, _t) = setup(&[]);
        let result = run_forced(&adapter, &manager, 10, false).unwrap();

        assert_eq!(result["embedding_space_committed"], json!(true));
        assert_eq!(stored_model(&adapter).as_deref(), Some("new"));
    }

    /// The index rebuilt for a new dimension keeps the old metric and HNSW
    /// parameters instead of falling back to cosine / defaults
    #[test]
    fn dimension_change_keeps_index_parameters() {
        let (adapter, _m, _t) = setup(&[Some("abc")]);
        adapter
            .create_vector_index("kb", "embedding", 2, "euclidean", 5_000, 24, 300, 80)
            .unwrap();

        handle_dimension_change(&adapter, "kb", "embedding", 3).unwrap();

        let indexes = adapter.list_vector_indexes("kb").unwrap();
        assert_eq!(indexes.len(), 1);
        let idx = &indexes[0];
        assert_eq!(idx["dim"], json!(3));
        assert_eq!(idx["metric"], json!("euclidean"));
        assert_eq!(idx["max_vectors"], json!(5_000));
        assert_eq!(idx["hnsw_params"]["m"], json!(24));
        assert_eq!(idx["hnsw_params"]["ef_construction"], json!(300));
        assert_eq!(idx["hnsw_params"]["ef_search"], json!(80));
    }
}
