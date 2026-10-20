//! CRUD tool handlers: find, insert, update, delete, count, distinct, aggregate
//!
//! Uses typed parameter structs for compile-time validation.
//! Supports auto-embedding: if enabled on a collection, embeddings are
//! automatically generated from source fields on insert/update.

use crate::adapter::{FindOptions, IronBaseAdapter};
use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use crate::scripting::ScriptLimits;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::helpers::{
    check_cancelled, parse_sort_value, validate_collection_name, validate_document,
    validate_filter, validate_update, DEFAULT_QUERY_LIMIT,
};
use super::params::{
    AggregateParams, CountParams, DeleteParams, DistinctParams, FindOneParams, FindParams,
    InsertManyParams, InsertOneParams, ParseParams, UpdateManyParams, UpdateOneParams,
};

/// Dispatch CRUD tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    match name {
        "insert_one" => handle_insert_one(params, adapter, embedding_manager),
        "insert_many" => handle_insert_many(params, adapter, embedding_manager),
        "find" => handle_find(params, adapter, limits, cancel_flag),
        "find_one" => handle_find_one(params, adapter),
        "update_one" => handle_update_one(params, adapter, embedding_manager),
        "update_many" => handle_update_many(params, adapter, embedding_manager),
        "delete_one" => handle_delete_one(params, adapter),
        "delete_many" => handle_delete_many(params, adapter),
        "count" => handle_count_documents(params, adapter),
        "distinct" => handle_distinct(params, adapter),
        "aggregate" => handle_aggregate(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown CRUD tool: {}",
            name
        ))),
    }
}

/// Apply auto-embedding to a document if configured for the collection
fn apply_auto_embedding(
    document: &mut Value,
    config: &ironbase_core::storage::AutoEmbeddingConfig,
    manager: &EmbeddingManager,
) -> Result<bool> {
    // Skip if target field already exists and skip_if_exists is true
    if config.skip_if_exists && document.get(&config.target_field).is_some() {
        return Ok(false);
    }

    // Get source text
    let source_text = match document.get(&config.source_field) {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(), // Convert non-strings to JSON string
        None => return Ok(false),         // No source field, skip
    };

    if source_text.is_empty() {
        return Ok(false);
    }

    // Generate embedding
    let provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!(
            "Embedding provider '{}' not available",
            config.provider
        ))
    })?;

    let embedding = provider
        .embed(&source_text)
        .map_err(|e| McpError::internal(format!("Embedding generation failed: {}", e)))?;

    // Validate dimension if specified
    if let Some(expected_dim) = config.dimension {
        if embedding.len() != expected_dim {
            return Err(McpError::internal(format!(
                "Embedding dimension mismatch: expected {}, got {}",
                expected_dim,
                embedding.len()
            )));
        }
    }

    // Set the embedding in the document
    if let Some(obj) = document.as_object_mut() {
        obj.insert(config.target_field.clone(), json!(embedding));
    }

    Ok(true)
}

/// Whether `update` writes `field`, or a path above or below it. A
/// replacement document (no `$` operators) writes every field.
fn update_touches_field(update: &Value, field: &str) -> bool {
    let Some(ops) = update.as_object() else {
        return false;
    };
    let related = |path: &str| {
        path == field
            || path.starts_with(&format!("{}.", field))
            || field.starts_with(&format!("{}.", path))
    };
    ops.iter().any(|(op, spec)| {
        if !op.starts_with('$') {
            return true;
        }
        spec.as_object().is_some_and(|paths| {
            paths.iter().any(|(path, value)| {
                // $rename also writes the new name
                related(path) || (op == "$rename" && value.as_str().is_some_and(related))
            })
        })
    })
}

/// The auto-embedding config of `collection` when `update` changes its
/// source field and does not write the target itself (a caller-supplied
/// embedding is kept). Updates used to leave the old embedding in place
/// (hybrid review F3).
fn reembed_config_for_update(
    adapter: &IronBaseAdapter,
    collection: &str,
    update: &Value,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Option<ironbase_core::storage::AutoEmbeddingConfig> {
    embedding_manager.as_ref()?;
    let config = adapter.get_auto_embedding_config(collection).ok()??;
    (config.enabled
        && update_touches_field(update, &config.source_field)
        && !update_touches_field(update, &config.target_field))
    .then_some(config)
}

/// Documents re-embedded per provider batch after an update
const REEMBED_BATCH: usize = 64;

/// Re-embed the documents `ids` from their current source field after an
/// update changed it. `skip_if_exists` does not apply: the existing
/// embedding was made from the old text. A document whose source is gone
/// or empty loses its embedding instead of keeping a stale one. Returns how
/// many documents were re-embedded.
fn refresh_auto_embeddings(
    adapter: &IronBaseAdapter,
    collection: &str,
    config: &ironbase_core::storage::AutoEmbeddingConfig,
    manager: &EmbeddingManager,
    ids: &[Value],
) -> Result<usize> {
    let provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!(
            "Embedding provider '{}' not available",
            config.provider
        ))
    })?;
    let mut refreshed = 0;
    for batch in ids.chunks(REEMBED_BATCH) {
        check_cancelled()?;
        let mut texts: Vec<(Value, String)> = Vec::new();
        for id in batch {
            let Some(doc) = adapter.find_one(collection, json!({"_id": id}))? else {
                continue; // deleted since
            };
            let text = match doc.get(&config.source_field) {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            if text.is_empty() {
                adapter.update_one(
                    collection,
                    json!({"_id": id}),
                    json!({"$unset": {&config.target_field: ""}}),
                )?;
            } else {
                texts.push((id.clone(), text));
            }
        }
        if texts.is_empty() {
            continue;
        }
        let refs: Vec<&str> = texts.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = provider
            .embed_batch(&refs)
            .map_err(|e| McpError::internal(format!("Embedding generation failed: {}", e)))?;
        for ((id, _), embedding) in texts.iter().zip(embeddings) {
            if let Some(expected_dim) = config.dimension {
                if embedding.len() != expected_dim {
                    return Err(McpError::internal(format!(
                        "Embedding dimension mismatch: expected {}, got {}",
                        expected_dim,
                        embedding.len()
                    )));
                }
            }
            adapter.update_one(
                collection,
                json!({"_id": id}),
                json!({"$set": {&config.target_field: embedding}}),
            )?;
            refreshed += 1;
        }
    }
    Ok(refreshed)
}

/// Re-embed after an update and report the outcome in `response`. The
/// update is already stored, so an embedding failure is reported, not
/// returned as the tool's error.
fn report_refresh(
    response: &mut Value,
    adapter: &IronBaseAdapter,
    collection: &str,
    config: &ironbase_core::storage::AutoEmbeddingConfig,
    manager: &EmbeddingManager,
    ids: &[Value],
) {
    match refresh_auto_embeddings(adapter, collection, config, manager, ids) {
        Ok(0) => {}
        Ok(n) => response["auto_embedded_count"] = json!(n),
        Err(e) => {
            tracing::warn!(
                "Auto-embedding after update failed in '{}': {}",
                collection,
                e
            );
            response["auto_embed_error"] = json!(e.to_string());
        }
    }
}

fn handle_insert_one(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    check_cancelled()?;

    let p: InsertOneParams = InsertOneParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_document(&p.document)?;

    let mut document = p.document;

    // Apply auto-embedding if configured
    let mut embedding_applied = false;
    if let Some(manager) = embedding_manager.as_ref() {
        if let Ok(Some(config)) = adapter.get_auto_embedding_config(&p.collection) {
            if config.enabled {
                embedding_applied = apply_auto_embedding(&mut document, &config, manager)?;
            }
        }
    }

    let id = adapter.insert_one(&p.collection, document)?;

    let mut response = json!({"inserted_id": id});
    if embedding_applied {
        response["auto_embedded"] = json!(true);
    }
    Ok(response)
}

fn handle_insert_many(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    check_cancelled()?;

    let p: InsertManyParams = InsertManyParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let mut documents = p.documents;
    let mut embeddings_applied = 0;

    // Apply auto-embedding if configured
    if let Some(manager) = embedding_manager.as_ref() {
        if let Ok(Some(config)) = adapter.get_auto_embedding_config(&p.collection) {
            if config.enabled {
                // Batch embedding for efficiency
                let provider = manager.get_provider(&config.provider);

                if let Some(provider) = provider {
                    // Collect texts that need embedding
                    let texts_with_indices: Vec<(usize, String)> = documents
                        .iter()
                        .enumerate()
                        .filter_map(|(i, doc)| {
                            // Skip if target exists and skip_if_exists is true
                            if config.skip_if_exists && doc.get(&config.target_field).is_some() {
                                return None;
                            }
                            // Get source text
                            match doc.get(&config.source_field) {
                                Some(Value::String(text)) if !text.is_empty() => {
                                    Some((i, text.clone()))
                                }
                                Some(other) => {
                                    let text = other.to_string();
                                    if !text.is_empty() {
                                        Some((i, text))
                                    } else {
                                        None
                                    }
                                }
                                None => None,
                            }
                        })
                        .collect();

                    if !texts_with_indices.is_empty() {
                        let text_refs: Vec<&str> =
                            texts_with_indices.iter().map(|(_, t)| t.as_str()).collect();

                        // Generate embeddings in batch
                        let embeddings = provider.embed_batch(&text_refs).map_err(|e| {
                            McpError::internal(format!("Batch embedding failed: {}", e))
                        })?;

                        // Apply embeddings to documents
                        for ((idx, _), embedding) in
                            texts_with_indices.iter().zip(embeddings.iter())
                        {
                            if let Some(obj) = documents[*idx].as_object_mut() {
                                obj.insert(config.target_field.clone(), json!(embedding));
                                embeddings_applied += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    let ids = adapter.insert_many(&p.collection, documents)?;

    let mut response = json!({
        "inserted_ids": ids,
        "inserted_count": ids.len()
    });
    if embeddings_applied > 0 {
        response["auto_embedded_count"] = json!(embeddings_applied);
    }
    Ok(response)
}

fn handle_find(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    limits: Option<&ScriptLimits>,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<Value> {
    // Check for cancellation before starting potentially slow operation
    check_cancelled()?;

    let p: FindParams = FindParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Use dynamic limit from ScriptLimits if available, otherwise default
    let max_limit = limits
        .map(|l| l.max_find_documents)
        .unwrap_or(DEFAULT_QUERY_LIMIT);

    // Apply limit: user's limit capped at max_limit, or max_limit if not specified
    let effective_limit = p.limit.map(|l| l.min(max_limit)).or(Some(max_limit));

    // Get max_result_size from ScriptLimits for OOM protection; without
    // ScriptLimits (stdio transport) fall back to the RAM-based default
    let max_response_bytes = Some(
        limits
            .map(|l| l.max_result_size)
            .unwrap_or_else(ironbase_core::calculate_safe_response_limit),
    );

    let options = FindOptions {
        projection: p.projection,
        sort: parse_sort_value(p.sort)?,
        limit: effective_limit,
        skip: p.skip,
        include_total: p.include_total,
        max_response_bytes,
        cancel_flag,
    };

    let result = adapter.find(&p.collection, p.query, options)?;
    let mut response = json!({
        "documents": result.documents,
        "count": result.documents.len()
    });
    if let Some(total) = result.total {
        response["total"] = json!(total);
    }
    Ok(response)
}

fn handle_find_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: FindOneParams = FindOneParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    // Build FindOptions with projection (core handles projection)
    let options = if let Some(proj) = p.projection {
        let proj_map: std::collections::HashMap<String, i32> = serde_json::from_value(proj)
            .map_err(|e| McpError::invalid_params(format!("Invalid projection format: {}", e)))?;
        ironbase_core::find_options::FindOptions::new().with_projection(proj_map)
    } else {
        ironbase_core::find_options::FindOptions::new()
    };

    // Core handles projection - consistent with find_with_options
    let document = adapter.find_one_with_options(&p.collection, p.query, options)?;
    Ok(json!({"document": document}))
}

fn handle_update_one(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    check_cancelled()?;

    let p: UpdateOneParams = UpdateOneParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    // When the source field changes, pin the update to the document it
    // matches now, so the re-embedded document is the one that changed
    let reembed = reembed_config_for_update(adapter, &p.collection, &p.update, embedding_manager);
    let mut filter = p.filter;
    let mut target_id = None;
    if reembed.is_some() {
        target_id = adapter
            .find_one(&p.collection, filter.clone())?
            .and_then(|doc| doc.get("_id").cloned());
        if let Some(ref id) = target_id {
            filter = json!({"$and": [filter, {"_id": id}]});
        }
    }

    // Use update_one_with_options for upsert support
    let result = adapter.update_one_with_options(&p.collection, filter, p.update, p.upsert)?;

    // Build response - include upserted_id if present
    let mut response = json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    });

    let mut embedding_applied = false;

    if let Some(ref upserted_id) = result.upserted_id {
        response["upserted_id"] = json!(upserted_id);

        // FIX: Apply auto-embedding to newly upserted document
        // When upsert creates a new document, it bypasses the insert_one handler
        // which normally applies auto-embedding. We need to do it here.
        if let Some(manager) = embedding_manager.as_ref() {
            if let Ok(Some(config)) = adapter.get_auto_embedding_config(&p.collection) {
                if config.enabled {
                    // Fetch the newly inserted document
                    let query = json!({"_id": upserted_id});
                    if let Ok(Some(mut doc)) = adapter.find_one(&p.collection, query.clone()) {
                        // Apply auto-embedding
                        if apply_auto_embedding(&mut doc, &config, manager).unwrap_or(false) {
                            // Update the document with the embedding
                            let update = json!({"$set": {&config.target_field: doc.get(&config.target_field)}});
                            let _ = adapter.update_one(&p.collection, query, update);
                            embedding_applied = true;
                        }
                    }
                }
            }
        }
    }

    if embedding_applied {
        response["auto_embedded"] = json!(true);
    }

    if let (Some(config), Some(id), Some(manager)) = (&reembed, target_id, embedding_manager) {
        if result.modified_count > 0 {
            report_refresh(
                &mut response,
                adapter,
                &p.collection,
                config,
                manager,
                &[id],
            );
        }
    }

    Ok(response)
}

fn handle_update_many(
    params: Value,
    adapter: &Arc<IronBaseAdapter>,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    check_cancelled()?;

    // Note: UpdateManyParams doesn't have upsert field - this is by design.
    // MongoDB supports upsert for updateMany, but our API explicitly doesn't
    // to prevent accidental bulk upserts. Use update_one with upsert=true instead.
    let p: UpdateManyParams = UpdateManyParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    // When the source field changes, note the matching documents first:
    // the update may make them stop matching the filter
    let reembed = reembed_config_for_update(adapter, &p.collection, &p.update, embedding_manager);
    let ids: Vec<Value> = if reembed.is_some() {
        let options = FindOptions {
            projection: Some(json!({"_id": 1})),
            ..Default::default()
        };
        adapter
            .find(&p.collection, p.filter.clone(), options)?
            .documents
            .into_iter()
            .filter_map(|doc| doc.get("_id").cloned())
            .collect()
    } else {
        Vec::new()
    };

    let result = adapter.update_many(&p.collection, p.filter, p.update)?;
    let mut response = json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    });
    if let (Some(config), Some(manager)) = (&reembed, embedding_manager) {
        if result.modified_count > 0 {
            report_refresh(&mut response, adapter, &p.collection, config, manager, &ids);
        }
    }
    Ok(response)
}

fn handle_delete_one(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DeleteParams = DeleteParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;

    let count = adapter.delete_one(&p.collection, p.filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_delete_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DeleteParams = DeleteParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;

    let count = adapter.delete_many(&p.collection, p.filter)?;
    Ok(json!({"deleted_count": count}))
}

fn handle_count_documents(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: CountParams = CountParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let count = adapter.count_documents(&p.collection, p.query)?;
    Ok(json!({"count": count}))
}

fn handle_distinct(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: DistinctParams = DistinctParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let all_values = adapter.distinct(&p.collection, &p.field, p.query)?;

    // Apply limit if specified (bounded by DEFAULT_QUERY_LIMIT for safety)
    let limit = p
        .limit
        .map(|l| l.min(DEFAULT_QUERY_LIMIT))
        .unwrap_or(all_values.len());
    let values: Vec<_> = all_values.into_iter().take(limit).collect();

    Ok(json!({"values": values, "count": values.len()}))
}

fn handle_aggregate(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: AggregateParams = AggregateParams::parse(params)?;
    validate_collection_name(&p.collection)?;

    let results = adapter.aggregate(&p.collection, p.pipeline)?;
    Ok(json!({"results": results, "count": results.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{EmbeddingProvider, EmbeddingResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// Embeds a text as [length, 1]; counts the texts it embedded
    struct CountingProvider(Arc<AtomicUsize>);

    impl EmbeddingProvider for CountingProvider {
        fn embed(&self, text: &str) -> EmbeddingResult<Vec<f32>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![text.chars().count() as f32, 1.0])
        }
        fn dimension(&self) -> usize {
            2
        }
        fn model_name(&self) -> &str {
            "counting"
        }
        fn provider_name(&self) -> &str {
            "counting"
        }
    }

    fn setup() -> (
        Arc<IronBaseAdapter>,
        Option<Arc<EmbeddingManager>>,
        Arc<AtomicUsize>,
        TempDir,
    ) {
        let tmp = TempDir::new().unwrap();
        let adapter = Arc::new(
            IronBaseAdapter::new(tmp.path().join("t.mlite").to_string_lossy().to_string()).unwrap(),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let mut manager = EmbeddingManager::new();
        manager.register_provider("counting", Arc::new(CountingProvider(calls.clone())));
        let manager = Some(Arc::new(manager));
        for (id, content) in [(1, "abc"), (2, "abcd"), (3, "xy")] {
            adapter
                .insert_one(
                    "kb",
                    json!({"_id": id, "group": "g", "content": content, "embedding": [0.0, 0.0]}),
                )
                .unwrap();
        }
        adapter
            .set_auto_embedding_config(
                "kb",
                Some(ironbase_core::storage::AutoEmbeddingConfig {
                    enabled: true,
                    source_field: "content".to_string(),
                    target_field: "embedding".to_string(),
                    provider: "counting".to_string(),
                    dimension: Some(2),
                    skip_if_exists: true,
                    ..Default::default()
                }),
            )
            .unwrap();
        (adapter, manager, calls, tmp)
    }

    fn embedding_of(adapter: &IronBaseAdapter, id: i64) -> Option<Value> {
        adapter
            .find_one("kb", json!({"_id": id}))
            .unwrap()
            .unwrap()
            .get("embedding")
            .cloned()
    }

    /// Hybrid review F3: an update that changes the source field re-embeds
    /// the document, even with skip_if_exists (the old vector is stale)
    #[test]
    fn update_one_reembeds_changed_source() {
        let (a, m, calls, _t) = setup();
        let out = handle_update_one(
            json!({"collection":"kb","filter":{"content":"abc"},"update":{"$set":{"content":"abcdefg"}}}),
            &a,
            &m,
        )
        .unwrap();
        assert_eq!(out["auto_embedded_count"], json!(1));
        assert_eq!(embedding_of(&a, 1), Some(json!([7.0, 1.0])));
        assert_eq!(embedding_of(&a, 2), Some(json!([0.0, 0.0])));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn update_without_source_change_does_not_embed() {
        let (a, m, calls, _t) = setup();
        handle_update_one(
            json!({"collection":"kb","filter":{"_id":1},"update":{"$set":{"group":"h"}}}),
            &a,
            &m,
        )
        .unwrap();
        // A caller-supplied embedding next to the new text is kept
        handle_update_one(
            json!({"collection":"kb","filter":{"_id":2},
                   "update":{"$set":{"content":"zz","embedding":[9.0, 9.0]}}}),
            &a,
            &m,
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedding_of(&a, 2), Some(json!([9.0, 9.0])));
    }

    #[test]
    fn update_many_reembeds_every_changed_document() {
        let (a, m, _calls, _t) = setup();
        // The update makes the documents stop matching the filter
        let out = handle_update_many(
            json!({"collection":"kb","filter":{"group":"g"},
                   "update":{"$set":{"content":"hello","group":"done"}}}),
            &a,
            &m,
        )
        .unwrap();
        assert_eq!(out["auto_embedded_count"], json!(3));
        for id in 1..=3 {
            assert_eq!(embedding_of(&a, id), Some(json!([5.0, 1.0])));
        }
    }

    #[test]
    fn unsetting_the_source_drops_the_embedding() {
        let (a, m, _calls, _t) = setup();
        handle_update_one(
            json!({"collection":"kb","filter":{"_id":3},"update":{"$unset":{"content":""}}}),
            &a,
            &m,
        )
        .unwrap();
        assert_eq!(embedding_of(&a, 3), None);
    }

    #[test]
    fn touches_field_paths() {
        assert!(update_touches_field(
            &json!({"$set":{"content":1}}),
            "content"
        ));
        assert!(update_touches_field(
            &json!({"$set":{"content.title":1}}),
            "content"
        ));
        assert!(update_touches_field(&json!({"$set":{"doc":1}}), "doc.text"));
        assert!(update_touches_field(
            &json!({"$rename":{"old":"content"}}),
            "content"
        ));
        assert!(!update_touches_field(
            &json!({"$set":{"contents":1}}),
            "content"
        ));
        assert!(update_touches_field(
            &json!({"content":"replacement"}),
            "content"
        ));
    }
}
