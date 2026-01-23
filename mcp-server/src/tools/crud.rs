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
    InsertManyParams, InsertOneParams, ParseParams, UpdateParams,
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
        "update_many" => handle_update_many(params, adapter),
        "delete_one" => handle_delete_one(params, adapter),
        "delete_many" => handle_delete_many(params, adapter),
        "count_documents" => handle_count_documents(params, adapter),
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
        None => return Ok(false), // No source field, skip
    };

    if source_text.is_empty() {
        return Ok(false);
    }

    // Generate embedding
    let provider = manager.get_provider(&config.provider).ok_or_else(|| {
        McpError::internal(format!("Embedding provider '{}' not available", config.provider))
    })?;

    let embedding = provider.embed(&source_text).map_err(|e| {
        McpError::internal(format!("Embedding generation failed: {}", e))
    })?;

    // Validate dimension if specified
    if let Some(expected_dim) = config.dimension {
        if embedding.len() != expected_dim {
            return Err(McpError::internal(format!(
                "Embedding dimension mismatch: expected {}, got {}",
                expected_dim, embedding.len()
            )));
        }
    }

    // Set the embedding in the document
    if let Some(obj) = document.as_object_mut() {
        obj.insert(config.target_field.clone(), json!(embedding));
    }

    Ok(true)
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
                        let text_refs: Vec<&str> = texts_with_indices.iter().map(|(_, t)| t.as_str()).collect();

                        // Generate embeddings in batch
                        let embeddings = provider.embed_batch(&text_refs).map_err(|e| {
                            McpError::internal(format!("Batch embedding failed: {}", e))
                        })?;

                        // Apply embeddings to documents
                        for ((idx, _), embedding) in texts_with_indices.iter().zip(embeddings.iter()) {
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

    // Get max_result_size from ScriptLimits for OOM protection
    let max_response_bytes = limits.map(|l| l.max_result_size);

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

    let p: UpdateParams = UpdateParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    // Use update_one_with_options for upsert support
    let result = adapter.update_one_with_options(&p.collection, p.filter, p.update, p.upsert)?;

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

    Ok(response)
}

fn handle_update_many(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    check_cancelled()?;

    let p: UpdateParams = UpdateParams::parse(params)?;
    validate_collection_name(&p.collection)?;
    validate_filter(&p.filter)?;
    validate_update(&p.update)?;

    // FIX: Reject upsert for update_many - not yet supported
    // MongoDB supports upsert for updateMany (creates one document if no matches),
    // but our core doesn't have update_many_with_options yet.
    if p.upsert {
        return Err(McpError::invalid_params(
            "upsert is not supported for update_many. Use update_one with upsert=true instead.",
        ));
    }

    let result = adapter.update_many(&p.collection, p.filter, p.update)?;
    Ok(json!({
        "matched_count": result.matched_count,
        "modified_count": result.modified_count
    }))
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
