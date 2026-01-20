//! Embedding generation tool handlers

use crate::embedding::EmbeddingManager;
use crate::error::{McpError, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

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

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch embedding tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    embedding_manager: &Option<Arc<EmbeddingManager>>,
) -> Result<Value> {
    // Check if embedding manager is available
    let manager = embedding_manager.as_ref().ok_or_else(|| {
        McpError::internal(
            "Embedding not available. Set IRONBASE_FASTTEXT_MODEL environment variable.",
        )
    })?;

    match name {
        "embed_text" => handle_embed_text(params, manager),
        "embed_batch" => handle_embed_batch(params, manager),
        "embed_list_models" => handle_embed_list_models(manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown embedding tool: {}",
            name
        ))),
    }
}

/// Check if a tool name is an embedding tool
pub fn is_embedding_tool(name: &str) -> bool {
    matches!(name, "embed_text" | "embed_batch" | "embed_list_models")
}

// ============================================================================
// Tool Handlers
// ============================================================================

fn handle_embed_text(params: Value, manager: &EmbeddingManager) -> Result<Value> {
    let p: EmbedTextParams = EmbedTextParams::parse(params)?;

    let vector = manager
        .embed(&p.text, p.provider.as_deref())
        .map_err(|e| McpError::internal(format!("Embedding failed: {}", e)))?;

    let provider_name = p
        .provider
        .as_deref()
        .unwrap_or(manager.default_provider_name());

    Ok(json!({
        "vector": vector,
        "dimension": vector.len(),
        "provider": provider_name,
        "model": manager.get_provider(provider_name)
            .map(|p| p.model_name().to_string())
            .unwrap_or_default()
    }))
}

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

    // Convert Vec<String> to Vec<&str>
    let text_refs: Vec<&str> = p.texts.iter().map(|s| s.as_str()).collect();

    let vectors = manager
        .embed_batch(&text_refs, p.provider.as_deref())
        .map_err(|e| McpError::internal(format!("Batch embedding failed: {}", e)))?;

    let provider_name = p
        .provider
        .as_deref()
        .unwrap_or(manager.default_provider_name());

    let dimension = vectors.first().map(|v| v.len()).unwrap_or(0);

    Ok(json!({
        "vectors": vectors,
        "count": vectors.len(),
        "dimension": dimension,
        "provider": provider_name,
        "model": manager.get_provider(provider_name)
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
