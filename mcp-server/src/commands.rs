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

/// Create document command
#[derive(Debug, Deserialize)]
pub struct CreateDocumentParams {
    pub document: Value,  // Full document as JSON
}

pub fn handle_create_document(
    adapter: &IronBaseAdapter,
    params: CreateDocumentParams,
) -> CommandResult {
    use mcp_docjl::Document;

    // Parse document from JSON
    let document: Document = serde_json::from_value(params.document)
        .map_err(|e| format!("Invalid document structure: {}", e))?;

    // Create document
    let document_id = adapter
        .create_document(document)
        .map_err(|e| format!("Failed to create document: {}", e))?;

    Ok(serde_json::json!({
        "success": true,
        "document_id": document_id
    }))
}

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
                "id": doc.identifier(),
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

/// Get specific section with children command (Phase 3.1)
#[derive(Debug, Deserialize)]
pub struct GetSectionParams {
    pub document_id: String,
    pub section_label: String,
    #[serde(default = "default_true")]
    pub include_subsections: bool,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
}

fn default_true() -> bool {
    true
}

fn default_max_depth() -> usize {
    10  // Reasonable default to prevent infinite recursion
}

pub fn handle_get_section(
    adapter: &IronBaseAdapter,
    params: GetSectionParams,
) -> CommandResult {
    let document = adapter
        .get_document(&params.document_id)
        .map_err(|e| format!("Failed to get document: {}", e))?;

    // Find the block with the specified label
    let section_block = find_block_by_label(&document.docjll, &params.section_label)
        .ok_or_else(|| format!("Section '{}' not found in document", params.section_label))?;

    // Clone the block and optionally limit depth
    let result_block = if params.include_subsections {
        limit_block_depth(section_block.clone(), params.max_depth)
    } else {
        // Return block without children
        strip_children(section_block.clone())
    };

    let section_value = serde_json::to_value(&result_block)
        .map_err(|e| format!("Failed to serialize section: {}", e))?;

    Ok(serde_json::json!({
        "section": section_value,
        "document_id": params.document_id,
        "label": params.section_label
    }))
}

/// Recursively search for a block by label
fn find_block_by_label<'a>(blocks: &'a [Block], label: &str) -> Option<&'a Block> {
    for block in blocks {
        if block.label() == Some(label) {
            return Some(block);
        }
        // Recursively search in children
        if let Some(children) = block.children() {
            if let Some(found) = find_block_by_label(children, label) {
                return Some(found);
            }
        }
    }
    None
}

/// Limit the depth of a block's children
fn limit_block_depth(block: Block, max_depth: usize) -> Block {
    if max_depth == 0 {
        return strip_children(block);
    }

    let mut result_block = block;
    if let Some(children) = result_block.children_mut() {
        *children = children
            .iter()
            .map(|child| limit_block_depth(child.clone(), max_depth - 1))
            .collect();
    }
    result_block
}

/// Remove all children from a block
fn strip_children(mut block: Block) -> Block {
    if let Some(children) = block.children_mut() {
        children.clear();
    }
    block
}

/// Estimate token count for document or section (Phase 3.2)
#[derive(Debug, Deserialize)]
pub struct EstimateTokensParams {
    pub document_id: String,
    #[serde(default)]
    pub section_label: Option<String>,
}

pub fn handle_estimate_tokens(
    adapter: &IronBaseAdapter,
    params: EstimateTokensParams,
) -> CommandResult {
    let document = adapter
        .get_document(&params.document_id)
        .map_err(|e| format!("Failed to get document: {}", e))?;

    // Get the blocks to estimate
    let blocks_to_estimate: &[Block] = if let Some(label) = &params.section_label {
        // Find specific section
        let section_block = find_block_by_label(&document.docjll, label)
            .ok_or_else(|| format!("Section '{}' not found in document", label))?;
        // Create a slice with just this block (we need to estimate it recursively)
        std::slice::from_ref(section_block)
    } else {
        // Estimate entire document
        &document.docjll
    };

    // Estimate tokens using a simple heuristic
    let token_estimate = estimate_tokens_for_blocks(blocks_to_estimate);

    // Calculate some useful stats
    let char_count = count_chars_in_blocks(blocks_to_estimate);
    let block_count = count_blocks_recursive(blocks_to_estimate);

    Ok(serde_json::json!({
        "document_id": params.document_id,
        "section_label": params.section_label,
        "estimated_tokens": token_estimate,
        "character_count": char_count,
        "block_count": block_count,
        "estimation_method": "GPT-style (chars/4 + blocks*20)"
    }))
}

/// Estimate tokens for blocks recursively
/// Uses a simple heuristic: ~4 chars per token + overhead for structure
fn estimate_tokens_for_blocks(blocks: &[Block]) -> usize {
    let char_count = count_chars_in_blocks(blocks);
    let block_count = count_blocks_recursive(blocks);

    // Heuristic:
    // - ~4 characters per token (GPT-style)
    // - ~20 tokens overhead per block for structure/labels
    (char_count / 4) + (block_count * 20)
}

/// Count total characters in blocks recursively
fn count_chars_in_blocks(blocks: &[Block]) -> usize {
    let mut count = 0;
    for block in blocks {
        // Serialize block to JSON and count characters
        if let Ok(json_str) = serde_json::to_string(block) {
            count += json_str.len();
        }

        // Recursively count children
        if let Some(children) = block.children() {
            count += count_chars_in_blocks(children);
        }
    }
    count
}

/// Count total number of blocks recursively
fn count_blocks_recursive(blocks: &[Block]) -> usize {
    let mut count = blocks.len();
    for block in blocks {
        if let Some(children) = block.children() {
            count += count_blocks_recursive(children);
        }
    }
    count
}

/// Search document content command
#[derive(Debug, Deserialize)]
pub struct SearchContentParams {
    pub document_id: String,
    pub query: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    100
}

pub fn handle_search_content(
    adapter: &IronBaseAdapter,
    params: SearchContentParams,
) -> CommandResult {
    // Helper function to extract text from paragraphs/headings
    fn extract_text_from_block(block: &Block) -> String {
        // Serialize to JSON and extract text content
        if let Ok(json) = serde_json::to_string(block) {
            json
        } else {
            String::new()
        }
    }

    let document = adapter
        .get_document(&params.document_id)
        .map_err(|e| format!("Failed to get document: {}", e))?;

    let query = if params.case_sensitive {
        params.query.clone()
    } else {
        params.query.to_lowercase()
    };

    let mut matches = Vec::new();
    let mut match_count = 0;

    // Search through all blocks
    for (block_index, block) in document.docjll.iter().enumerate() {
        if match_count >= params.max_results {
            break;
        }

        let mut block_text = String::new();
        let mut block_label = None;
        let mut block_type = String::new();

        // Serialize block to JSON for searching
        block_text = extract_text_from_block(block);
        block_label = block.label().map(|s| s.to_string());
        block_type = format!("{:?}", block.block_type());

        // Perform search
        let search_text = if params.case_sensitive {
            block_text.clone()
        } else {
            block_text.to_lowercase()
        };

        if search_text.contains(&query) {
            matches.push(serde_json::json!({
                "block_index": block_index,
                "block_type": block_type,
                "label": block_label,
                "text": block_text,
                "block": block,
            }));
            match_count += 1;
        }
    }

    Ok(serde_json::json!({
        "document_id": document.identifier(),
        "query": params.query,
        "total_matches": matches.len(),
        "matches": matches,
    }))
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
        "start" => InsertPosition::Start,
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
        "start" => InsertPosition::Start,
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
    pub label: Option<String>,  // Exact label match
    pub label_prefix: Option<String>,
    pub level: Option<u8>,  // Filter by heading level (1-6)
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
        label: params.query.label,
        label_prefix: params.query.label_prefix,
        level: params.query.level,
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
        "mcp_docjl_create_document" => {
            let params: CreateDocumentParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_create_document(adapter, params)
        }
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
        "mcp_docjl_search_content" => {
            let params: SearchContentParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_search_content(adapter, params)
        }
        "mcp_docjl_get_section" => {
            let params: GetSectionParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_get_section(adapter, params)
        }
        "mcp_docjl_estimate_tokens" => {
            let params: EstimateTokensParams = serde_json::from_value(params)
                .map_err(|e| format!("Invalid parameters: {}", e))?;
            handle_estimate_tokens(adapter, params)
        }
        _ => Err(format!("Unknown command: {}", method)),
    }
}
