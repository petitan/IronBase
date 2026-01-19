//! RAG (Retrieval Augmented Generation) tool handlers
//!
//! Provides semantic search capabilities using FastText embeddings and HNSW indexing.

use crate::adapter::IronBaseAdapter;
use crate::error::{McpError, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::helpers::verify_admin_key_opt;
use super::params::{
    ParseParams, RagCollectionCreateParams, RagCollectionDeleteParams, RagCollectionNameParams,
    RagDocumentDeleteParams, RagDocumentImportParams, RagSearchParams,
};

/// SECURITY: Validate collection/document name to prevent path traversal attacks
fn validate_name(name: &str, field_name: &str) -> Result<()> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(McpError::invalid_params(format!(
            "{} cannot contain path separators (/, \\) or '..'",
            field_name
        )));
    }
    Ok(())
}

/// Dispatch RAG tool calls
pub fn dispatch(name: &str, params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    match name {
        "rag_collection_create" => handle_collection_create(params, adapter),
        "rag_collection_list" => handle_collection_list(adapter),
        "rag_collection_stats" => handle_collection_stats(params, adapter),
        "rag_collection_delete" => handle_collection_delete(params, adapter),
        "rag_document_import" => handle_document_import(params, adapter),
        "rag_document_list" => handle_document_list(params, adapter),
        "rag_document_delete" => handle_document_delete(params, adapter),
        "rag_search" => handle_search(params, adapter),
        _ => Err(McpError::invalid_params(format!(
            "Unknown RAG tool: {}",
            name
        ))),
    }
}

/// Create a new RAG collection
fn handle_collection_create(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagCollectionCreateParams = RagCollectionCreateParams::parse(params)?;

    // Verify admin key
    verify_admin_key_opt(p.admin_key.as_deref())?;

    // Validate name
    if p.name.is_empty() || p.name.len() > 64 {
        return Err(McpError::invalid_params(
            "Collection name must be 1-64 characters",
        ));
    }
    validate_name(&p.name, "Collection name")?;

    // Create RAG collection
    adapter.rag_collection_create(
        &p.name,
        &p.model_path,
        p.chunk_max_tokens.unwrap_or(1000),
        p.chunk_overlap.unwrap_or(100),
    )?;

    Ok(json!({
        "success": true,
        "collection": p.name,
        "message": format!("RAG collection '{}' created successfully", p.name)
    }))
}

/// List all RAG collections
fn handle_collection_list(adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let collections = adapter.rag_collection_list()?;

    Ok(json!({
        "collections": collections,
        "count": collections.len()
    }))
}

/// Get RAG collection statistics
fn handle_collection_stats(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagCollectionNameParams = RagCollectionNameParams::parse(params)?;
    let name = p
        .get_name()
        .ok_or_else(|| McpError::invalid_params("Missing 'name' or 'collection' parameter"))?;

    // SECURITY: Prevent path traversal
    validate_name(name, "Collection name")?;

    let stats = adapter.rag_collection_stats(name)?;

    Ok(json!(stats))
}

/// Delete a RAG collection
fn handle_collection_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagCollectionDeleteParams = RagCollectionDeleteParams::parse(params)?;

    // Verify admin key
    verify_admin_key_opt(p.admin_key.as_deref())?;

    // SECURITY: Prevent path traversal
    validate_name(&p.name, "Collection name")?;

    adapter.rag_collection_delete(&p.name)?;

    Ok(json!({
        "success": true,
        "message": format!("RAG collection '{}' deleted", p.name)
    }))
}

/// Import a document into a RAG collection
fn handle_document_import(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagDocumentImportParams = RagDocumentImportParams::parse(params)?;

    // SECURITY: Prevent path traversal
    validate_name(&p.collection, "Collection name")?;
    validate_name(&p.doc_id, "Document ID")?;

    // Validate inputs
    if p.doc_id.is_empty() {
        return Err(McpError::invalid_params("doc_id cannot be empty"));
    }
    if p.title.is_empty() {
        return Err(McpError::invalid_params("title cannot be empty"));
    }
    if p.content.is_empty() {
        return Err(McpError::invalid_params("content cannot be empty"));
    }

    let result = adapter.rag_document_import(&p.collection, &p.doc_id, &p.title, &p.content)?;

    Ok(json!(result))
}

/// List documents in a RAG collection
fn handle_document_list(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagCollectionNameParams = RagCollectionNameParams::parse(params)?;
    let collection = p
        .get_name()
        .ok_or_else(|| McpError::invalid_params("Missing 'collection' parameter"))?;

    // SECURITY: Prevent path traversal
    validate_name(collection, "Collection name")?;

    let documents = adapter.rag_document_list(collection)?;

    Ok(json!({
        "documents": documents,
        "count": documents.len()
    }))
}

/// Delete a document from a RAG collection
fn handle_document_delete(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagDocumentDeleteParams = RagDocumentDeleteParams::parse(params)?;

    // SECURITY: Prevent path traversal
    validate_name(&p.collection, "Collection name")?;
    validate_name(&p.doc_id, "Document ID")?;

    let deleted = adapter.rag_document_delete(&p.collection, &p.doc_id)?;

    Ok(json!({
        "success": deleted,
        "message": if deleted {
            format!("Document '{}' deleted from '{}'", p.doc_id, p.collection)
        } else {
            format!("Document '{}' not found in '{}'", p.doc_id, p.collection)
        }
    }))
}

/// Search for relevant chunks in a RAG collection
fn handle_search(params: Value, adapter: &Arc<IronBaseAdapter>) -> Result<Value> {
    let p: RagSearchParams = RagSearchParams::parse(params)?;

    // SECURITY: Prevent path traversal
    validate_name(&p.collection, "Collection name")?;

    // Validate query
    if p.query.trim().is_empty() {
        return Err(McpError::invalid_params("Search query cannot be empty"));
    }

    let results = adapter.rag_search(&p.collection, &p.query, p.limit.unwrap_or(5), p.min_score)?;

    Ok(json!({
        "results": results,
        "count": results.len(),
        "query": p.query
    }))
}
