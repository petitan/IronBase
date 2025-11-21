// MCP command handlers

use mcp_docjl::{
    DeleteOptions, DocumentOperations, InsertOptions, IronBaseAdapter, MoveOptions,
    SearchQuery, Block, InsertPosition, BlockType, AuditQuery, read_audit_log,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// Command execution result
pub type CommandResult = Result<Value, String>;

/// List documents command
#[derive(Debug, Deserialize)]
pub struct ListDocumentsParams {
    #[serde(default)]
    pub filter: Option<Value>,
}

pub fn handle_list_documents(
    adapter: &IronBaseAdapter,
    _params: ListDocumentsParams,
) -> CommandResult {
    let documents = adapter
        .list_documents()
        .map_err(|e| format!("Failed to list documents: {}", e))?;

    // Convert to summary format
    let summaries: Vec<Value> = documents
        .iter()
        .map(|doc| {
            serde_json::json!({
                "id": doc.id,
                "title": doc.metadata.title,
                "version": doc.metadata.version,
                "blocks_count": doc.metadata.blocks_count,
                "modified_at": doc.metadata.modified_at,
            })
        })
        .collect();

    Ok(serde_json::json!({ "documents": summaries }))
}

/// Get document command
#[derive(Debug, Deserialize)]
pub struct GetDocumentParams {
    pub document_id: String,
    #[serde(default)]
    pub sections: Option<Vec<String>>,
    #[serde(default)]
    pub depth: Option<usize>,
}

pub fn handle_get_document(
    adapter: &IronBaseAdapter,
    params: GetDocumentParams,
) -> CommandResult {
    let document = adapter
        .get_document(&params.document_id)
        .map_err(|e| format!("Failed to get document: {}", e))?;

    // TODO: Filter by sections and depth if specified
    let doc_value = serde_json::to_value(&document)
        .map_err(|e| format!("Failed to serialize document: {}", e))?;

    Ok(serde_json::json!({ "document": doc_value }))
}

/// Insert block command
#[derive(Debug, Deserialize)]
pub struct InsertBlockParams {
    pub document_id: String,
    pub block: Value,
    #[serde(default)]
    pub parent_label: Option<String>,
    #[serde(default = "default_position")]
    pub position: String,
    #[serde(default)]
    pub anchor_label: Option<String>,
}

fn default_position() -> String {
    "end".to_string()
}

pub fn handle_insert_block(
    adapter: &mut IronBaseAdapter,
    params: InsertBlockParams,
) -> CommandResult {
    // Deserialize block
    let block: Block = serde_json::from_value(params.block)
        .map_err(|e| format!("Invalid block format: {}", e))?;

    // Parse position
    let position = match params.position.as_str() {
        "before" => InsertPosition::Before,
        "after" => InsertPosition::After,
        "inside" => InsertPosition::Inside,
        "end" => InsertPosition::End,
        _ => {
            return Err(format!("Invalid position: {}", params.position));
        }
    };

    let options = InsertOptions {
        parent_label: params.parent_label,
        position,
        anchor_label: params.anchor_label,
        auto_label: true,
        validate: true,
    };

    let result = adapter
        .insert_block(&params.document_id, block, options)
        .map_err(|e| format!("Failed to insert block: {}", e))?;

    let response = serde_json::json!({
        "success": result.success,
        "block_label": result.affected_labels.first().map(|c| &c.new_label),
        "audit_id": result.audit_id,
        "affected_labels": result.affected_labels,
        "warnings": result.warnings,
    });

    Ok(response)
}

/// Update block command
#[derive(Debug, Deserialize)]
pub struct UpdateBlockParams {
    pub document_id: String,
    pub block_label: String,
    pub updates: HashMap<String, Value>,
}

pub fn handle_update_block(
    adapter: &mut IronBaseAdapter,
    params: UpdateBlockParams,
) -> CommandResult {
    let result = adapter
        .update_block(&params.document_id, &params.block_label, params.updates)
        .map_err(|e| format!("Failed to update block: {}", e))?;

    let response = serde_json::json!({
        "success": result.success,
        "audit_id": result.audit_id,
        "affected_labels": result.affected_labels,
    });

    Ok(response)
}

/// Move block command
#[derive(Debug, Deserialize)]
pub struct MoveBlockParams {
    pub document_id: String,
    pub block_label: String,
    #[serde(default)]
    pub target_parent: Option<String>,
    #[serde(default = "default_position")]
    pub position: String,
}

pub fn handle_move_block(adapter: &mut IronBaseAdapter, params: MoveBlockParams) -> CommandResult {
    let position = match params.position.as_str() {
        "before" => InsertPosition::Before,
        "after" => InsertPosition::After,
        "inside" => InsertPosition::Inside,
        "end" => InsertPosition::End,
        _ => {
            return Err(format!("Invalid position: {}", params.position));
        }
    };

    let options = MoveOptions {
        target_parent: params.target_parent,
        position,
        update_references: true,
        renumber_labels: true,
    };

    let result = adapter
        .move_block(&params.document_id, &params.block_label, options)
        .map_err(|e| format!("Failed to move block: {}", e))?;

    let response = serde_json::json!({
        "success": result.success,
        "audit_id": result.audit_id,
        "affected_labels": result.affected_labels,
    });

    Ok(response)
}

/// Delete block command
#[derive(Debug, Deserialize)]
pub struct DeleteBlockParams {
    pub document_id: String,
    pub block_label: String,
    #[serde(default)]
    pub cascade: bool,
    #[serde(default)]
    pub force: bool,
}

pub fn handle_delete_block(
    adapter: &mut IronBaseAdapter,
    params: DeleteBlockParams,
) -> CommandResult {
    let options = DeleteOptions {
        cascade: params.cascade,
        check_references: true,
        force: params.force,
    };

    let result = adapter
        .delete_block(&params.document_id, &params.block_label, options)
        .map_err(|e| format!("Failed to delete block: {}", e))?;

    let response = serde_json::json!({
        "success": result.success,
        "audit_id": result.audit_id,
        "deleted_count": result.affected_labels.len(),
    });

    Ok(response)
}

/// List headings command
#[derive(Debug, Deserialize)]
pub struct ListHeadingsParams {
    pub document_id: String,
    #[serde(default)]
    pub max_depth: Option<usize>,
}

pub fn handle_list_headings(
    adapter: &IronBaseAdapter,
    params: ListHeadingsParams,
) -> CommandResult {
    let outline = adapter
        .get_outline(&params.document_id, params.max_depth)
        .map_err(|e| format!("Failed to get outline: {}", e))?;

    let outline_value = serde_json::to_value(&outline)
        .map_err(|e| format!("Failed to serialize outline: {}", e))?;

    Ok(serde_json::json!({ "outline": outline_value }))
}

/// Search blocks command
#[derive(Debug, Deserialize)]
pub struct SearchBlocksParams {
    pub document_id: String,
    #[serde(default)]
    pub query: SearchQueryParams,
}

#[derive(Debug, Default, Deserialize)]
pub struct SearchQueryParams {
    #[serde(rename = "type")]
    pub block_type: Option<String>,
    pub content_contains: Option<String>,
    pub has_label: Option<bool>,
    pub has_compliance_note: Option<bool>,
    pub label_prefix: Option<String>,
}

pub fn handle_search_blocks(
    adapter: &IronBaseAdapter,
    params: SearchBlocksParams,
) -> CommandResult {
    // Convert to domain SearchQuery
    let block_type = params.query.block_type.as_ref().and_then(|t| {
        serde_json::from_str::<BlockType>(&format!("\"{}\"", t)).ok()
    });

    let query = SearchQuery {
        block_type,
        content_contains: params.query.content_contains,
        has_label: params.query.has_label,
        has_compliance_note: params.query.has_compliance_note,
        label_prefix: params.query.label_prefix,
    };

    let results = adapter
        .search_blocks(&params.document_id, query)
        .map_err(|e| format!("Failed to search blocks: {}", e))?;

    let results_value = serde_json::to_value(&results)
        .map_err(|e| format!("Failed to serialize results: {}", e))?;

    Ok(serde_json::json!({ "results": results_value }))
}

/// Validate references command
#[derive(Debug, Deserialize)]
pub struct ValidateReferencesParams {
    pub document_id: String,
}

pub fn handle_validate_references(
    adapter: &IronBaseAdapter,
    params: ValidateReferencesParams,
) -> CommandResult {
    let result = adapter
        .validate_references(&params.document_id)
        .map_err(|e| format!("Failed to validate references: {}", e))?;

    let result_value = serde_json::to_value(&result)
        .map_err(|e| format!("Failed to serialize result: {}", e))?;

    Ok(result_value)
}

/// Validate schema command
#[derive(Debug, Deserialize)]
pub struct ValidateSchemaParams {
    pub document_id: String,
}

pub fn handle_validate_schema(
    adapter: &IronBaseAdapter,
    params: ValidateSchemaParams,
) -> CommandResult {
    let result = adapter
        .validate_schema(&params.document_id)
        .map_err(|e| format!("Failed to validate schema: {}", e))?;

    let result_value = serde_json::to_value(&result)
        .map_err(|e| format!("Failed to serialize result: {}", e))?;

    Ok(result_value)
}

/// Get audit log command
#[derive(Debug, Deserialize)]
pub struct GetAuditLogParams {
    pub document_id: Option<String>,
    pub block_label: Option<String>,
    pub limit: Option<usize>,
}

pub fn handle_get_audit_log(
    audit_logger_path: &std::path::PathBuf,
    params: GetAuditLogParams,
) -> CommandResult {
    let mut query = AuditQuery::new();

    if let Some(doc_id) = params.document_id {
        query = query.document(doc_id);
    }

    if let Some(label) = params.block_label {
        query = query.block(label);
    }

    if let Some(limit) = params.limit {
        query = query.limit(limit);
    }

    let entries = read_audit_log(audit_logger_path, query)
        .map_err(|e| format!("Failed to read audit log: {}", e))?;

    let entries_value = serde_json::to_value(&entries)
        .map_err(|e| format!("Failed to serialize entries: {}", e))?;

    Ok(serde_json::json!({ "entries": entries_value }))
}

/// Dispatch command to appropriate handler
pub fn dispatch_command(
    method: &str,
    params: Value,
    adapter: &mut IronBaseAdapter,
    audit_log_path: &std::path::PathBuf,
) -> CommandResult {
    match method {
        "mcp_docjl_list_documents" => {
            let params: ListDocumentsParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_list_documents(adapter, params)
        }
        "mcp_docjl_get_document" => {
            let params: GetDocumentParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_get_document(adapter, params)
        }
        "mcp_docjl_insert_block" => {
            let params: InsertBlockParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_insert_block(adapter, params)
        }
        "mcp_docjl_update_block" => {
            let params: UpdateBlockParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_update_block(adapter, params)
        }
        "mcp_docjl_move_block" => {
            let params: MoveBlockParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_move_block(adapter, params)
        }
        "mcp_docjl_delete_block" => {
            let params: DeleteBlockParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_delete_block(adapter, params)
        }
        "mcp_docjl_list_headings" => {
            let params: ListHeadingsParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_list_headings(adapter, params)
        }
        "mcp_docjl_search_blocks" => {
            let params: SearchBlocksParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_search_blocks(adapter, params)
        }
        "mcp_docjl_validate_references" => {
            let params: ValidateReferencesParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_validate_references(adapter, params)
        }
        "mcp_docjl_validate_schema" => {
            let params: ValidateSchemaParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_validate_schema(adapter, params)
        }
        "mcp_docjl_get_audit_log" => {
            let params: GetAuditLogParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_get_audit_log(audit_log_path, params)
        }
        _ => Err(format!("Unknown command: {}", method)),
    }
}
