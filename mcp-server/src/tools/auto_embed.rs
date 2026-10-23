//! Auto-embedding tool handlers
//!
//! Provides MCP tools for configuring automatic embedding generation on collections.

use crate::adapter::IronBaseAdapter;
use crate::embedding::{EmbeddingManager, EmbeddingProvider};
use crate::error::{McpError, Result};
use crate::jobs::{JobManager, JobType};
use ironbase_core::storage::{AutoEmbeddingConfig, ChunkingConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
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

    // Get dimension from provider if not specified. The provider decides the
    // vector size, so a different explicit one is accepted only when its output
    // confirms it (its configured dimension may be a default): an index of
    // another dimension would skip every vector (review 2026-10-17 T3).
    let dimension = match p.dimension {
        Some(requested) if requested != provider.dimension() => {
            let actual = provider
                .embed("dimension check")
                .map_err(|e| {
                    McpError::internal(format!(
                        "Cannot check dimension {} against provider '{}': {}",
                        requested, provider_name, e
                    ))
                })?
                .len();
            if actual != requested {
                return Err(McpError::invalid_params(format!(
                    "dimension {} does not match provider '{}', which returns {}-dimensional vectors",
                    requested, provider_name, actual
                )));
            }
            requested
        }
        Some(requested) => requested,
        None => provider.dimension(),
    };

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
        reembed_pending: false,
    };

    // Vectors already in the target field that another provider, model,
    // preprocessing, dimension or source field produced must be re-embedded,
    // not kept next to new ones (review 2026-10-17 F4/T3). A legacy config
    // without a model name leaves the model unknown, so only the other
    // differences force (as at startup); a re-embed still pending always does.
    let previous = adapter
        .get_auto_embedding_config(&p.collection)?
        .filter(|prev| prev.target_field == config.target_field);
    let index_dim_changed = vector_index_dim(adapter, &p.collection, &config.target_field)
        .is_some_and(|d| d != dimension);
    let has_vectors = adapter.count_documents(
        &p.collection,
        json!({ &config.target_field: { "$exists": true } }),
    )? > 0;
    let config_changed = previous.as_ref().is_some_and(|prev| {
        prev.provider != config.provider
            || prev
                .model
                .as_ref()
                .is_some_and(|m| Some(m) != config.model.as_ref())
            || prev
                .preprocessing_version
                .as_ref()
                .is_some_and(|v| Some(v) != config.preprocessing_version.as_ref())
            || prev.dimension.is_some_and(|d| d != dimension)
            || prev.source_field != config.source_field
    });
    let space_changed = previous.as_ref().is_some_and(|prev| prev.reembed_pending)
        || (has_vectors && (config_changed || index_dim_changed));

    // A backfill still running embeds with the previous config: stop it, so
    // it cannot overwrite the vectors of the one started below
    cancel_backfill_jobs(job_manager, &p.collection);

    if space_changed {
        // The stored model/preprocessing keep describing the stored vectors
        // until the forced re-embed records the new ones. Saved before the
        // old vectors go: an interrupted dimension change leaves an index
        // that already matches, which only the pending marker tells apart.
        let mut stored = config.clone();
        stored.model = previous.as_ref().and_then(|prev| prev.model.clone());
        stored.preprocessing_version = previous
            .as_ref()
            .and_then(|prev| prev.preprocessing_version.clone());
        stored.reembed_pending = true;
        adapter.set_auto_embedding_config(&p.collection, Some(stored))?;
    }
    // A different dimension: the old index and vectors go
    if index_dim_changed {
        handle_dimension_change(adapter, &p.collection, &config.target_field, dimension)
            .map_err(McpError::internal)?;
    }
    if !space_changed {
        // Save to collection metadata
        adapter.set_auto_embedding_config(&p.collection, Some(config.clone()))?;
    }

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

    // Count existing documents that need embedding: missing target_field, or
    // all of them for a forced re-embed
    let backfill_query = if space_changed {
        json!({})
    } else {
        json!({ &config.target_field: { "$exists": false } })
    };
    let existing_count = adapter
        .count_documents(&p.collection, backfill_query)
        .unwrap_or(0);

    // Auto-start backfill if there are existing documents without embeddings,
    // or a forced re-embed of all of them into the new space
    let backfill_job_id = if space_changed {
        start_backfill_job(
            adapter,
            manager,
            &config,
            &p.collection,
            job_manager,
            true, // force=true: re-embed every document into the new space
        )
    } else if existing_count > 0 {
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

    if space_changed {
        response["reembed"] = json!(true);
        response["message"] = json!(
            "Auto-embedding enabled with a different provider, model, preprocessing, dimension \
             or source field: all existing vectors are being re-embedded (retried on startup \
             until done)."
        );
    }
    if let Some(ref job_id) = backfill_job_id {
        response["backfill_job_id"] = json!(job_id);
        if !space_changed {
            response["message"] =
                json!("Auto-embedding enabled. Backfill job started for existing documents.");
        }
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
                },
                "reembed_pending": config.reembed_pending
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

/// Dimension of the vector index on `field`, if there is one
/// Cancel the backfill jobs of `collection` that have not finished
fn cancel_backfill_jobs(job_manager: &Option<Arc<JobManager>>, collection: &str) {
    let Some(job_mgr) = job_manager.as_ref() else {
        return;
    };
    for job in job_mgr.list_jobs() {
        if matches!(&job.job_type, JobType::EmbedBackfill { collection: c, .. } if c == collection)
            && job_mgr.cancel_job(&job.id)
        {
            tracing::info!(
                "Backfill job {} for '{}' cancelled: auto-embed reconfigured",
                job.id,
                collection
            );
        }
    }
}

fn vector_index_dim(adapter: &IronBaseAdapter, collection: &str, field: &str) -> Option<usize> {
    adapter
        .list_vector_indexes(collection)
        .ok()?
        .iter()
        .find_map(|idx| {
            if idx.get("field")?.as_str()? == field {
                idx.get("dim")?.as_u64().map(|d| d as usize)
            } else {
                None
            }
        })
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
/// collection's auto-embed config only after it handled every document (see
/// `settle_failed_documents` for the ones it could not embed). Until then the
/// stored values keep describing the old vectors, so a run that is interrupted,
/// fails or never starts is detected and restarted by the next startup check
/// instead of leaving a mixed embedding space.
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
    // Forced run: documents that may still hold an old-model vector (embedding
    // failed, or no source text); settled once the pass is complete
    let mut failed_ids: Vec<Value> = Vec::new();
    // Forced run: ids already handled. The pass pages with skip over the
    // `_id`-ordered catalog, and a delete before the cursor shifts the rest
    // left; each page therefore re-reads one batch before the cursor and skips
    // what it already handled.
    let mut visited: HashSet<String> = HashSet::new();
    if force && visited.try_reserve(total_count).is_err() {
        job_manager.fail_job(
            job_id,
            format!(
                "Cannot allocate re-embed state for {} documents",
                total_count
            ),
        );
        return;
    }
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

        let (page_start, page_limit) = if force {
            (skip_offset.saturating_sub(batch_size), batch_size * 2)
        } else {
            (0, batch_size)
        };
        let find_options = crate::adapter::FindOptions {
            limit: Some(page_limit),
            skip: if force { Some(page_start) } else { None },
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

        if force && page_start > 0 && !visited.contains(&id_key(&result.documents[0])) {
            // More documents before the cursor were deleted than one batch
            // covers: back off another batch until the page starts in
            // documents this pass already handled
            skip_offset = page_start;
            continue;
        }

        // Forced run: only the documents this pass has not handled yet
        let documents: Vec<&Value> = if force {
            result
                .documents
                .iter()
                .filter(|doc| visited.insert(id_key(doc)))
                .collect()
        } else {
            result.documents.iter().collect()
        };

        // Extract texts from source field
        let texts: Vec<&str> = documents
            .iter()
            .filter_map(|doc| doc.get(&config.source_field).and_then(|v| v.as_str()))
            .collect();

        if force {
            // A vector without source text cannot be re-embedded either
            for doc in &documents {
                let has_source = doc
                    .get(&config.source_field)
                    .and_then(|v| v.as_str())
                    .is_some();
                if !has_source && doc.get(&config.target_field).is_some() {
                    if let Some(id) = doc.get("_id") {
                        failed_ids.push(id.clone());
                    }
                }
            }
        }

        let batch_had_updates = if !texts.is_empty() {
            // Generate embeddings — try batch first, fallback to individual on failure
            let docs_with_source: Vec<_> = documents
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
            // A cancelled job writes nothing more: a newer one may own the field
            let store_one = |id: &Value, embedding: &[f32]| -> bool {
                !job_manager.is_cancelled(job_id)
                    && store_embedding(adapter, collection, &config.target_field, id, embedding)
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
            if !documents.is_empty() {
                tracing::warn!(
                    "Batch of {} documents has no valid source field '{}' - skipping",
                    documents.len(),
                    config.source_field
                );
            }
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
            skip_offset = page_start + result.documents.len();
        }

        // If we got a short page, we're done
        if result.documents.len() < page_limit {
            traversed_all = true;
            break;
        }
    }

    // A cancel can also land inside the last batch's per-document fallback
    if job_manager.is_cancelled(job_id) {
        job_manager.update_progress(job_id, processed, Some(total_count), "Cancelled");
        return;
    }

    let mut summary = json!({
        "processed": processed,
        "errors": errors,
        "total": total_count,
        "provider": config.provider
    });

    if force {
        let settled = if traversed_all {
            settle_failed_documents(
                job_id,
                job_manager,
                adapter,
                provider.as_ref(),
                config,
                collection,
                &failed_ids,
                shutdown_flag,
            )
        } else {
            None
        };
        let committed = settled.is_some() && commit_embedding_space(adapter, collection, config);
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
        summary["stale_vectors_removed"] = json!(settled.unwrap_or(0));
    }

    // Complete the job
    job_manager.complete_job(job_id, summary);
}

/// Key of a document's `_id` in the forced pass's visited set
fn id_key(doc: &Value) -> String {
    doc.get("_id").map(|id| id.to_string()).unwrap_or_default()
}

/// Store one embedding into a document via update_one
fn store_embedding(
    adapter: &IronBaseAdapter,
    collection: &str,
    target_field: &str,
    id: &Value,
    embedding: &[f32],
) -> bool {
    let filter = json!({ "_id": id });
    let update = json!({
        "$set": { target_field: embedding }
    });
    match adapter.update_one(collection, filter, update) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("Failed to update document {:?}: {}", id, e);
            false
        }
    }
}

/// Settle the documents a complete forced pass could not re-embed, so no
/// old-model vector stays in the new space. Each one is re-read (a concurrent
/// update may have changed or re-embedded it) and its current source text is
/// tried once more; the vector of a document that still cannot be embedded is
/// removed.
///
/// Returns the number of vectors removed, or None when the run has to count as
/// incomplete: the provider fails a health check (an outage, not documents it
/// cannot embed), the job is stopped, or a read or write fails.
#[allow(clippy::too_many_arguments)]
fn settle_failed_documents(
    job_id: &str,
    job_manager: &JobManager,
    adapter: &IronBaseAdapter,
    provider: &dyn EmbeddingProvider,
    config: &AutoEmbeddingConfig,
    collection: &str,
    ids: &[Value],
    shutdown_flag: &std::sync::atomic::AtomicBool,
) -> Option<usize> {
    if ids.is_empty() {
        return Some(0);
    }
    if let Err(e) = provider.embed("health check") {
        tracing::warn!(
            "Re-embed of '{}': provider fails a health check ({}); {} failed documents keep \
             their vectors",
            collection,
            e,
            ids.len()
        );
        return None;
    }

    let mut removed = 0;
    for id in ids {
        if job_manager.is_cancelled(job_id)
            || shutdown_flag.load(std::sync::atomic::Ordering::SeqCst)
        {
            return None;
        }
        let doc = match adapter.find_one(collection, json!({ "_id": id })) {
            Ok(Some(doc)) => doc,
            Ok(None) => continue, // deleted meanwhile
            Err(e) => {
                tracing::warn!("Failed to re-read document {:?}: {}", id, e);
                return None;
            }
        };
        if let Some(text) = doc.get(&config.source_field).and_then(|v| v.as_str()) {
            if let Ok(embedding) = provider.embed(text) {
                if !store_embedding(adapter, collection, &config.target_field, id, &embedding) {
                    return None;
                }
                continue;
            }
        }
        if doc.get(&config.target_field).is_none() {
            continue;
        }
        let update = json!({ "$unset": { &config.target_field: "" } });
        if let Err(e) = adapter.update_one(collection, json!({ "_id": id }), update) {
            tracing::warn!("Failed to remove stale vector of {:?}: {}", id, e);
            return None;
        }
        removed += 1;
    }
    if removed > 0 {
        tracing::warn!(
            "Re-embed of '{}': {} documents cannot be embedded with the new model; their old \
             vectors were removed",
            collection,
            removed
        );
    }
    Some(removed)
}

/// Save the model and preprocessing version a forced re-embed produced into the
/// collection's auto-embed config. Skipped when any other setting of the config
/// changed while the job ran (disabled, other provider, source or target field).
fn commit_embedding_space(
    adapter: &IronBaseAdapter,
    collection: &str,
    target: &AutoEmbeddingConfig,
) -> bool {
    // Everything but the embedding space itself must still match the job's config
    let settings = |c: &AutoEmbeddingConfig| {
        let mut c = c.clone();
        c.model = None;
        c.preprocessing_version = None;
        c.reembed_pending = false;
        serde_json::to_value(c).ok()
    };
    let mut current = match adapter.get_auto_embedding_config(collection) {
        Ok(Some(c)) if settings(&c).is_some() && settings(&c) == settings(target) => c,
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
    current.reembed_pending = false;
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

        let mut config = match adapter.get_auto_embedding_config(collection) {
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
        let saved_dim = vector_index_dim(adapter, collection, &config.target_field).unwrap_or(0);

        let dimension_changed = saved_dim > 0 && saved_dim != current_dim;

        let was_pending = config.reembed_pending;
        if dimension_changed {
            tracing::info!(
                "Dimension change detected for '{}': {} → {} on field '{}', rebuilding HNSW index",
                collection,
                saved_dim,
                current_dim,
                config.target_field
            );
            // Persist the pending marker before the old vectors go: an
            // interrupted rebuild leaves an index that already matches the
            // provider, which only the marker tells apart. The stored
            // dimension (which inserts are validated against) follows the
            // provider.
            config.reembed_pending = true;
            if config.dimension.is_some() {
                config.dimension = Some(current_dim);
            }
            if let Err(e) = adapter.set_auto_embedding_config(collection, Some(config.clone())) {
                tracing::warn!("Failed to update config for '{}': {}", collection, e);
            }
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

        let pending = config.reembed_pending;
        if saved_model.is_empty()
            && saved_preproc.is_empty()
            && !force_reembed
            && !dimension_changed
            && !pending
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
            || pending
        {
            // Something changed or force requested — start re-embed
            let reason = if force_reembed {
                "force flag".to_string()
            } else if dimension_changed
                && saved_model == current_model
                && saved_preproc == current_preproc
            {
                format!("dimension {} → {}", saved_dim, current_dim)
            } else if was_pending
                && saved_model == current_model
                && saved_preproc == current_preproc
            {
                "an earlier re-embed did not complete".to_string()
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
            target_config.reembed_pending = false;

            // Persist the pending marker first: if the re-embed is interrupted
            // or never starts, the next startup restarts it even when names
            // alone cannot tell (e.g. a dimension change, same model name)
            if !pending {
                config.reembed_pending = true;
                if let Err(e) = adapter.set_auto_embedding_config(collection, Some(config.clone()))
                {
                    tracing::warn!("Failed to update config for '{}': {}", collection, e);
                }
            }

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
    use crate::embedding::{EmbeddingError, EmbeddingResult};
    use crate::jobs::JobStatus;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// Embeds a text as [length, 2.0] under model "new". Always fails on "bad",
    /// on "flaky" for its first `flaky_failures` calls, and on everything when
    /// `down`. With `deleter`, its n-th call deletes documents 0..8 of "kb"
    /// (a concurrent delete before the job's cursor).
    #[derive(Default)]
    struct NewModelProvider {
        dim: usize,
        down: bool,
        flaky_failures: AtomicUsize,
        calls: AtomicUsize,
        deleter: Option<(usize, Arc<IronBaseAdapter>)>,
    }

    impl NewModelProvider {
        fn new() -> Self {
            Self {
                dim: 2,
                ..Default::default()
            }
        }
    }

    impl EmbeddingProvider for NewModelProvider {
        fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((at, adapter)) = &self.deleter {
                if call == *at {
                    for id in 0..8 {
                        adapter.delete_one("kb", json!({"_id": id})).unwrap();
                    }
                }
            }
            let left = self.flaky_failures.load(Ordering::SeqCst);
            let flaky = text == "flaky" && left > 0;
            if flaky {
                self.flaky_failures.store(left - 1, Ordering::SeqCst);
            }
            if self.down || text == "bad" || flaky {
                return Err(EmbeddingError::ApiError("boom".to_string()));
            }
            let mut v = vec![0.0f32; self.dim];
            v[0] = text.chars().count() as f32;
            v[1] = 2.0;
            Ok(v)
        }
        fn dimension(&self) -> usize {
            self.dim
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
        setup_with(contents, |_| NewModelProvider::new())
    }

    fn setup_with(
        contents: &[Option<&str>],
        provider: impl FnOnce(&Arc<IronBaseAdapter>) -> NewModelProvider,
    ) -> (Arc<IronBaseAdapter>, Arc<EmbeddingManager>, TempDir) {
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
        manager.register_provider("mock", Arc::new(provider(&adapter)));
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
    /// must not trip the no-progress stop before the end of the collection.
    /// Their old vectors cannot be re-embedded, so they are removed.
    #[test]
    fn forced_reembed_passes_documents_without_source() {
        let mut contents = vec![None; 12];
        contents.push(Some("abc"));
        let (adapter, manager, _t) = setup(&contents);
        let result = run_forced(&adapter, &manager, 1, false).unwrap();

        assert_eq!(result["processed"], json!(1));
        assert_eq!(result["stale_vectors_removed"], json!(12));
        assert_eq!(result["embedding_space_committed"], json!(true));
        assert_eq!(stored_model(&adapter).as_deref(), Some("new"));
        let embedded = adapter
            .count_documents("kb", json!({"embedding": {"$exists": true}}))
            .unwrap();
        assert_eq!(embedded, 1);
    }

    /// Deletes before the cursor shift the skip window; the pass must still
    /// reach every remaining document (here more deletes than one batch)
    #[test]
    fn forced_reembed_reaches_all_documents_despite_deletes() {
        let texts: Vec<String> = (0..40).map(|i| format!("t{}", i)).collect();
        let contents: Vec<Option<&str>> = texts.iter().map(|t| Some(t.as_str())).collect();
        let (adapter, manager, _t) = setup_with(&contents, |adapter| NewModelProvider {
            deleter: Some((9, adapter.clone())),
            ..NewModelProvider::new()
        });
        let result = run_forced(&adapter, &manager, 4, false).unwrap();

        assert_eq!(result["embedding_space_committed"], json!(true));
        let docs = adapter
            .find("kb", json!({}), Default::default())
            .unwrap()
            .documents;
        assert_eq!(docs.len(), 32);
        for doc in docs {
            assert_eq!(doc["embedding"][1], json!(2.0), "{}", doc);
        }
    }

    /// A document that failed during the pass is retried with its current
    /// source before its vector is removed
    #[test]
    fn forced_reembed_retries_failed_documents() {
        let (adapter, manager, _t) = setup_with(&[Some("flaky"), Some("abc")], |_| {
            let p = NewModelProvider::new();
            p.flaky_failures.store(2, Ordering::SeqCst); // batch call + fallback
            p
        });
        let result = run_forced(&adapter, &manager, 10, false).unwrap();

        assert_eq!(result["errors"], json!(1));
        assert_eq!(result["stale_vectors_removed"], json!(0));
        assert_eq!(result["embedding_space_committed"], json!(true));
        let flaky = adapter.find_one("kb", json!({"_id": 0})).unwrap().unwrap();
        assert_eq!(flaky["embedding"], json!([5.0, 2.0]));
    }

    /// A provider outage on a collection smaller than the no-progress stop
    /// still reaches the end; the health check keeps it from removing vectors
    #[test]
    fn forced_reembed_during_outage_keeps_vectors() {
        let (adapter, manager, _t) = setup_with(&[Some("abc"), Some("de")], |_| NewModelProvider {
            down: true,
            ..NewModelProvider::new()
        });
        let result = run_forced(&adapter, &manager, 10, false).unwrap();

        assert_eq!(result["embedding_space_committed"], json!(false));
        assert_eq!(result["stale_vectors_removed"], json!(0));
        assert_eq!(stored_model(&adapter).as_deref(), Some("old"));
        let embedded = adapter
            .count_documents("kb", json!({"embedding": {"$exists": true}}))
            .unwrap();
        assert_eq!(embedded, 2);
    }

    /// The new model is not recorded over a config whose other settings
    /// changed while the job ran
    #[test]
    fn commit_rejects_changed_config() {
        let (adapter, _m, _t) = setup(&[Some("abc")]);
        let mut target = adapter.get_auto_embedding_config("kb").unwrap().unwrap();
        target.model = Some("new".to_string());
        target.source_field = "other".to_string();

        assert!(!commit_embedding_space(&adapter, "kb", &target));
        assert_eq!(stored_model(&adapter).as_deref(), Some("old"));
    }

    fn stored_config(adapter: &IronBaseAdapter) -> AutoEmbeddingConfig {
        adapter.get_auto_embedding_config("kb").unwrap().unwrap()
    }

    /// Wait (bounded) until the background re-embed recorded the new model
    fn wait_for_commit(adapter: &IronBaseAdapter) {
        for _ in 0..500 {
            let c = stored_config(adapter);
            if c.model.as_deref() == Some("new") && !c.reembed_pending {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("re-embed did not complete: {:?}", stored_config(adapter));
    }

    /// D1: the pending marker is stored before the re-embed starts, so a
    /// re-embed that never started is still known at the next startup
    #[test]
    fn startup_marks_reembed_pending() {
        let (adapter, manager, _t) = setup(&[Some("abc")]);
        check_model_changes_and_reembed(&adapter, &Some(manager), &None, false);

        let config = stored_config(&adapter);
        assert!(config.reembed_pending);
        assert_eq!(config.model.as_deref(), Some("old"));
    }

    /// D1: a pending re-embed is restarted even when the stored names already
    /// match the provider (e.g. an interrupted dimension change)
    #[test]
    fn startup_restarts_pending_reembed_with_matching_names() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de")]);
        let mut config = stored_config(&adapter);
        config.model = Some("new".to_string());
        config.reembed_pending = true;
        adapter
            .set_auto_embedding_config("kb", Some(config))
            .unwrap();

        let jobs = Arc::new(JobManager::new());
        check_model_changes_and_reembed(&adapter, &Some(manager), &Some(jobs.clone()), false);
        wait_for_commit(&adapter);
        jobs.shutdown();

        for doc in adapter
            .find("kb", json!({}), Default::default())
            .unwrap()
            .documents
        {
            assert_eq!(doc["embedding"][1], json!(2.0), "{}", doc);
        }
    }

    /// T3: enabling auto-embed with another model on a collection that
    /// already has vectors re-embeds all of them; until done the stored
    /// model stays the old one with the pending marker set
    #[test]
    fn enable_with_new_model_reembeds_existing_vectors() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de"), Some("f")]);
        let jobs = Arc::new(JobManager::new());
        let response = handle_auto_embed_enable(
            json!({"collection": "kb", "source_field": "content",
                   "target_field": "embedding", "provider": "mock"}),
            &adapter,
            &Some(manager),
            &Some(jobs.clone()),
        )
        .unwrap();
        assert_eq!(response["reembed"], json!(true), "{}", response);

        wait_for_commit(&adapter);
        jobs.shutdown();
        for doc in adapter
            .find("kb", json!({}), Default::default())
            .unwrap()
            .documents
        {
            assert_eq!(doc["embedding"][1], json!(2.0), "{}", doc);
        }
    }

    /// T3: re-enabling with the same model only backfills missing vectors
    #[test]
    fn enable_with_same_model_keeps_existing_vectors() {
        let (adapter, manager, _t) = setup(&[Some("abc")]);
        let mut config = stored_config(&adapter);
        config.model = Some("new".to_string());
        adapter
            .set_auto_embedding_config("kb", Some(config))
            .unwrap();

        let response = handle_auto_embed_enable(
            json!({"collection": "kb", "source_field": "content",
                   "target_field": "embedding", "provider": "mock"}),
            &adapter,
            &Some(manager),
            &None,
        )
        .unwrap();

        assert!(response.get("reembed").is_none(), "{}", response);
        assert!(!stored_config(&adapter).reembed_pending);
        let doc = adapter.find_one("kb", json!({"_id": 0})).unwrap().unwrap();
        assert_eq!(doc["embedding"], json!([0.0, 0.0]), "untouched");
    }

    fn enable_mock(
        adapter: &Arc<IronBaseAdapter>,
        manager: Arc<EmbeddingManager>,
        extra: Value,
        jobs: &Option<Arc<JobManager>>,
    ) -> Result<Value> {
        let mut params = json!({"collection": "kb", "source_field": "content",
                                "target_field": "embedding", "provider": "mock"});
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        handle_auto_embed_enable(params, adapter, &Some(manager), jobs)
    }

    /// T3: another provider with the same model name is another space; the
    /// response counts every document the forced re-embed covers
    #[test]
    fn enable_with_new_provider_reembeds() {
        let (adapter, manager, _t) = setup(&[Some("abc"), Some("de")]);
        let mut config = stored_config(&adapter);
        config.provider = "other".to_string();
        config.model = Some("new".to_string());
        adapter
            .set_auto_embedding_config("kb", Some(config))
            .unwrap();

        let response = enable_mock(&adapter, manager, json!({}), &None).unwrap();

        assert_eq!(response["reembed"], json!(true), "{}", response);
        assert_eq!(response["existing_documents"], json!(2), "{}", response);
        let config = stored_config(&adapter);
        assert!(config.reembed_pending);
        assert_eq!(config.provider, "mock");
    }

    /// T3: a dimension the provider does not return is rejected before
    /// anything changes
    #[test]
    fn enable_rejects_dimension_the_provider_does_not_return() {
        let (adapter, manager, _t) = setup(&[Some("abc")]);
        adapter
            .create_vector_index("kb", "embedding", 2, "cosine", 1_000, 16, 200, 50)
            .unwrap();

        let err = enable_mock(&adapter, manager, json!({"dimension": 5}), &None).unwrap_err();

        assert!(err.to_string().contains("2-dimensional"), "{}", err);
        assert_eq!(stored_config(&adapter).model.as_deref(), Some("old"));
        assert_eq!(
            adapter.list_vector_indexes("kb").unwrap()[0]["dim"],
            json!(2)
        );
        let doc = adapter.find_one("kb", json!({"_id": 0})).unwrap().unwrap();
        assert_eq!(doc["embedding"], json!([0.0, 0.0]));
    }

    /// T3: a pending re-embed stays pending and forced even when no vector
    /// is left in the field
    #[test]
    fn enable_keeps_pending_reembed_without_vectors() {
        let (adapter, manager, _t) = setup(&[Some("abc")]);
        adapter
            .update_many("kb", json!({}), json!({"$unset": {"embedding": ""}}))
            .unwrap();
        let mut config = stored_config(&adapter);
        config.reembed_pending = true;
        adapter
            .set_auto_embedding_config("kb", Some(config))
            .unwrap();

        let response = enable_mock(&adapter, manager, json!({}), &None).unwrap();

        assert_eq!(response["reembed"], json!(true), "{}", response);
        assert!(stored_config(&adapter).reembed_pending);
    }

    /// T3: enabling again cancels the backfill still running for the
    /// collection, so it cannot overwrite the new job's vectors
    #[test]
    fn enable_cancels_running_backfill() {
        let (adapter, manager, _t) = setup(&[Some("abc")]);
        let jobs = Arc::new(JobManager::new());
        let (old_job, _) = jobs.create_job(JobType::EmbedBackfill {
            collection: "kb".to_string(),
            provider: "mock".to_string(),
        });
        let (other_job, _) = jobs.create_job(JobType::EmbedBackfill {
            collection: "other".to_string(),
            provider: "mock".to_string(),
        });

        enable_mock(&adapter, manager, json!({}), &Some(jobs.clone())).unwrap();

        assert!(jobs.is_cancelled(&old_job));
        assert!(!jobs.is_cancelled(&other_job));
        jobs.shutdown();
    }

    /// T3: a new dimension rebuilds the vector index (keeping its metric)
    /// before the re-embed
    #[test]
    fn enable_with_new_dimension_rebuilds_index_and_reembeds() {
        let (adapter, manager, _t) = setup_with(&[Some("abc")], |_| NewModelProvider {
            dim: 3,
            ..NewModelProvider::new()
        });
        adapter
            .create_vector_index("kb", "embedding", 2, "euclidean", 1_000, 16, 200, 50)
            .unwrap();
        let jobs = Arc::new(JobManager::new());
        handle_auto_embed_enable(
            json!({"collection": "kb", "source_field": "content",
                   "target_field": "embedding", "provider": "mock"}),
            &adapter,
            &Some(manager),
            &Some(jobs.clone()),
        )
        .unwrap();

        wait_for_commit(&adapter);
        jobs.shutdown();
        let index = &adapter.list_vector_indexes("kb").unwrap()[0];
        assert_eq!(index["dim"], json!(3));
        assert_eq!(index["metric"], json!("euclidean"));
        let doc = adapter.find_one("kb", json!({"_id": 0})).unwrap().unwrap();
        assert_eq!(doc["embedding"], json!([3.0, 2.0, 0.0]));
        assert_eq!(stored_config(&adapter).dimension, Some(3));
    }

    /// After a dimension change the stored dimension (which inserts are
    /// validated against) follows the provider; the model stays old until the
    /// re-embed completes
    #[test]
    fn startup_dimension_change_updates_stored_dimension() {
        let (adapter, manager, _t) = setup_with(&[Some("abc")], |_| NewModelProvider {
            dim: 3,
            ..NewModelProvider::new()
        });
        adapter
            .create_vector_index("kb", "embedding", 2, "cosine", 1_000, 16, 200, 50)
            .unwrap();
        check_model_changes_and_reembed(&adapter, &Some(manager), &None, false);

        let config = adapter.get_auto_embedding_config("kb").unwrap().unwrap();
        assert_eq!(config.dimension, Some(3));
        assert_eq!(config.model.as_deref(), Some("old"));
        assert_eq!(
            adapter.list_vector_indexes("kb").unwrap()[0]["dim"],
            json!(3)
        );
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
