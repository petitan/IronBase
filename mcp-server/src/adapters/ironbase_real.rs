// Real IronBase storage adapter for DOCJL documents

use crate::domain::{
    Block, CrossReference, DeleteOptions, Document, DocumentOperations, DomainError,
    DomainResult, InsertOptions, InsertPosition, LabelGenerator,
    MoveOptions, OperationResult, OutlineItem, ReferenceValidator, SchemaValidator,
    SearchQuery, SearchResult, ValidationResult, LabelChange, ChangeReason,
};
use ironbase_core::DatabaseCore;
use ironbase_core::storage::StorageEngine;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Real IronBase adapter for DOCJL documents
pub struct RealIronBaseAdapter {
    db: Arc<RwLock<DatabaseCore<StorageEngine>>>,
    collection_name: String,
    label_generator: Arc<RwLock<LabelGenerator>>,
    cross_ref: Arc<RwLock<CrossReference>>,
    schema_validator: Arc<RwLock<SchemaValidator>>,
}

impl RealIronBaseAdapter {
    fn parse_document_value(raw: serde_json::Value) -> DomainResult<Document> {
        let mut doc: Document = serde_json::from_value(raw.clone())
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to parse document: {}", e),
            })?;

        if doc.db_id.is_none() {
            doc.db_id = raw.get("_id").cloned();
        }

        if doc.id.is_none() {
            if let Some(Value::String(id)) = raw.get("id") {
                doc.id = Some(id.clone());
            } else if let Some(db_id) = doc.db_id_as_string() {
                doc.id = Some(db_id);
            }
        }

        Ok(doc)
    }
    /// Create a new adapter with real IronBase
    pub fn new(path: PathBuf, collection_name: String) -> DomainResult<Self> {
        // Debug: print the absolute path being opened
        let abs_path = std::fs::canonicalize(&path)
            .unwrap_or_else(|_| path.clone());
        eprintln!("🔍 DEBUG: Opening database at: {:?}", abs_path);
        eprintln!("🔍 DEBUG: File exists: {}", abs_path.exists());
        eprintln!("🔍 DEBUG: File size: {} bytes", std::fs::metadata(&abs_path).map(|m| m.len()).unwrap_or(0));
        eprintln!("🔍 DEBUG: Collection name: {}", collection_name);

        // Open IronBase database
        let db = DatabaseCore::open(&path)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to open IronBase: {}", e),
            })?;

        let db = Arc::new(RwLock::new(db));

        // Collection will be created automatically on first insert
        // No need to manually create it here

        let adapter = Self {
            db,
            collection_name,
            label_generator: Arc::new(RwLock::new(LabelGenerator::new())),
            cross_ref: Arc::new(RwLock::new(CrossReference::new())),
            schema_validator: Arc::new(RwLock::new(SchemaValidator::default())),
        };

        // Initialize by scanning existing documents
        adapter.initialize()?;

        Ok(adapter)
    }

    /// Load all documents and build indexes
    fn initialize(&self) -> DomainResult<()> {
        eprintln!("🔍 DEBUG: Starting initialization...");
        let db = self.db.read();

        // List all collections
        let collections = db.list_collections();
        eprintln!("🔍 DEBUG: Found {} collections: {:?}", collections.len(), collections);

        // Try to get collection - it might not exist yet (created on first insert)
        let collection = match db.collection(&self.collection_name) {
            Ok(coll) => {
                eprintln!("🔍 DEBUG: Successfully got '{}' collection", self.collection_name);
                coll
            }
            Err(e) => {
                eprintln!("🔍 DEBUG: Collection '{}' doesn't exist yet: {}", self.collection_name, e);
                // Collection doesn't exist yet, skip initialization
                return Ok(());
            }
        };

        // Create secondary index on "id" field for O(log N) lookups
        // This allows us to efficiently find documents by semantic ID
        // Ignore errors if index already exists
        let _ = collection.create_index("id".to_string(), true);

        // Count documents first
        let count = collection.count_documents(&serde_json::json!({}))
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to count documents: {}", e),
            })?;
        eprintln!("🔍 DEBUG: Collection has {} documents", count);

        // Find all documents
        eprintln!("🔍 DEBUG: Calling find({{}})...");
        let docs_json = collection.find(&serde_json::json!({}))
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to find documents: {}", e),
            })?;
        eprintln!("🔍 DEBUG: find() returned {} documents", docs_json.len());

        let mut label_gen = self.label_generator.write();
        let mut cross_ref = self.cross_ref.write();

        // Process each document
        for doc_json in docs_json {
            let doc = Self::parse_document_value(doc_json)?;
            // Extract labels (skip duplicates across documents)
            for label in doc.collect_labels() {
                if !label_gen.exists(&label) {
                    label_gen.register(&label)?;
                }
            }

            // Extract cross-references
            for block in &doc.docjll {
                self.extract_references_recursive(block, &mut cross_ref);
            }
        }

        Ok(())
    }

    /// Extract references from a block recursively
    fn extract_references_recursive(&self, block: &Block, cross_ref: &mut CrossReference) {
        if let Some(label) = block.label() {
            cross_ref.register_label(label.to_string());

            let refs = block.extract_references();
            for target in refs {
                cross_ref.add_reference(label.to_string(), target);
            }
        }

        if let Some(children) = block.children() {
            for child in children {
                self.extract_references_recursive(child, cross_ref);
            }
        }
    }

    /// Get a document by ID from IronBase (supports both _id and semantic id field lookup)
    pub fn get_document(&self, document_id: &str) -> DomainResult<Document> {
        let db = self.db.read();
        let collection = db.collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        // Try to parse document_id as integer for _id lookup
        let id_value = document_id.parse::<i64>()
            .map(|n| serde_json::json!(n))
            .unwrap_or_else(|_| serde_json::json!(document_id));

        // First try lookup by _id field
        let query = serde_json::json!({"_id": id_value});
        let mut docs = collection.find(&query)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to find document by _id: {}", e),
            })?;

        // If not found by _id, try lookup by semantic "id" field (O(log N) via secondary index)
        if docs.is_empty() {
            let query = serde_json::json!({"id": document_id});
            docs = collection.find(&query)
                .map_err(|e| DomainError::StorageError {
                    message: format!("Failed to find document by semantic id: {}", e),
                })?;
        }

        // Fallback: If index query didn't work (index might be empty for existing docs),
        // do full scan and manual filter by "id" field
        if docs.is_empty() {
            let all_docs = collection.find(&serde_json::json!({}))
                .map_err(|e| DomainError::StorageError {
                    message: format!("Failed to scan documents: {}", e),
                })?;

            // Manual filter by "id" field
            for doc in all_docs {
                if let Some(doc_id) = doc.get("id").and_then(|v| v.as_str()) {
                    if doc_id == document_id {
                        docs.push(doc);
                        break;
                    }
                }
            }
        }

        if docs.is_empty() {
            return Err(DomainError::StorageError {
                message: format!("Document not found: {}", document_id),
            });
        }

        Self::parse_document_value(docs[0].clone())
    }

    /// Save a document to IronBase
    fn save_document(&self, document: &Document) -> DomainResult<()> {
        let db = self.db.read();
        let collection = db.collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        // Serialize document to JSON
        let mut doc_value = serde_json::to_value(document)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to serialize document: {}", e),
            })?;

        // Determine DB identifier (_id)
        let db_identifier = if let Some(id_value) = &document.db_id {
            id_value.clone()
        } else if let Some(semantic_id) = document.id.as_deref() {
            if let Ok(id_num) = semantic_id.parse::<i64>() {
                serde_json::json!(id_num)
            } else {
                let query = serde_json::json!({"id": semantic_id});
                let docs = collection.find(&query)
                    .map_err(|e| DomainError::StorageError {
                        message: format!("Failed to find document by semantic id: {}", e),
                    })?;

                if docs.is_empty() {
                    return Err(DomainError::StorageError {
                        message: format!("Document not found by semantic ID: {}", semantic_id),
                    });
                }

                docs[0]
                    .get("_id")
                    .cloned()
                    .ok_or_else(|| DomainError::StorageError {
                        message: format!("Document missing _id field: {}", semantic_id),
                    })?
            }
        } else {
            return Err(DomainError::StorageError {
                message: "Document missing id and _id".to_string(),
            });
        };

        doc_value["_id"] = db_identifier.clone();

        // Use IronBase update_one with $set operator to replace document
        let query = serde_json::json!({"_id": db_identifier.clone()});
        let update = serde_json::json!({"$set": doc_value});

        let (matched, _) = collection.update_one(&query, &update)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to update document: {}", e),
            })?;
        if matched == 0 {
            let doc_id = document
                .identifier()
                .unwrap_or_else(|| "<unknown>".to_string());
            return Err(DomainError::StorageError {
                message: format!("Document not found: {}", doc_id),
            });
        }

        Ok(())
    }

    /// Create a new document
    pub fn create_document(&self, document: Document) -> DomainResult<String> {
        let document_id = document.id.clone();

        // Serialize document to JSON
        let doc_value = serde_json::to_value(&document)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to serialize document: {}", e),
            })?;

        // Convert to HashMap (required by IronBase insert_one)
        let doc_map: HashMap<String, Value> = serde_json::from_value(doc_value)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to convert document to map: {}", e),
            })?;

        // Get collection (auto-creates if doesn't exist) and insert
        let collection = self.db.read().collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        // Insert document
        collection.insert_one(doc_map)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to insert document: {}", e),
            })?;

        // CRITICAL: Flush database to persist collection metadata
        // Without this, collection metadata is not saved and documents are lost on restart
        self.db.read().flush()
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to flush database: {}", e),
            })?;

        // Return the document ID
        Ok(document_id.unwrap_or_else(|| "<no id>".to_string()))
    }

    pub fn list_documents(&self) -> DomainResult<Vec<Document>> {
        let db = self.db.read();
        let collection = db.collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        let docs_json = collection.find(&serde_json::json!({}))
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to find documents: {}", e),
            })?;

        let mut documents = Vec::new();
        for doc_json in docs_json {
            documents.push(Self::parse_document_value(doc_json)?);
        }

        Ok(documents)
    }

    /// Generate audit ID
    fn generate_audit_id(&self) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        format!("op_{}", timestamp)
    }

    /// Insert block into document at specified position (NESTED format - uses children arrays)
    fn insert_block_into_document(
        &self,
        document: &mut Document,
        mut block: Block,
        options: &InsertOptions,
    ) -> DomainResult<(String, ChangeReason)> {
        // Auto-generate label if needed, track whether it was generated or user-provided
        let (block_label, change_reason) = if block.label().is_none() && options.auto_label {
            let prefix = crate::domain::label::default_prefix_for_type(
                &format!("{:?}", block.block_type()).to_lowercase(),
            );
            let label = self.label_generator.write().generate(prefix);
            block.set_label(label.clone());
            (label, ChangeReason::Generated)
        } else {
            (block.label().unwrap_or("unlabeled").to_string(), ChangeReason::UserProvided)
        };

        // Check for duplicate labels (only for user-provided labels)
        if change_reason == ChangeReason::UserProvided {
            if Self::label_exists_in_blocks(&document.docjll, &block_label) {
                return Err(DomainError::DuplicateLabel {
                    label: block_label,
                });
            }
        }

        // Validate if requested
        if options.validate {
            let validator = self.schema_validator.read();
            let validation_result = validator.validate_block(&block);
            if !validation_result.valid {
                return Err(DomainError::ValidationFailed {
                    errors: validation_result
                        .errors
                        .iter()
                        .map(|e| e.message.clone())
                        .collect(),
                });
            }
        }

        // Insert based on position (recursive for nested structure)
        match options.position {
            InsertPosition::Start => {
                document.docjll.insert(0, block.clone());
            }

            InsertPosition::End => {
                document.docjll.push(block.clone());
            }

            InsertPosition::Before => {
                let anchor = options.anchor_label.as_ref()
                    .ok_or_else(|| DomainError::InvalidOperation {
                        reason: "Before position requires anchor_label".to_string(),
                    })?;

                Self::insert_before_recursive(&mut document.docjll, anchor, block.clone())?;
            }

            InsertPosition::After => {
                let anchor = options.anchor_label.as_ref()
                    .ok_or_else(|| DomainError::InvalidOperation {
                        reason: "After position requires anchor_label".to_string(),
                    })?;

                Self::insert_after_recursive(&mut document.docjll, anchor, block.clone())?;
            }

            InsertPosition::Inside => {
                // NESTED format: Inside means adding to parent's children array
                let parent = options.parent_label.as_ref()
                    .ok_or_else(|| DomainError::InvalidOperation {
                        reason: "Inside position requires parent_label".to_string(),
                    })?;

                Self::insert_inside_recursive(&mut document.docjll, parent, block.clone())?;
            }
        }

        // Register label and references
        let mut cross_ref = self.cross_ref.write();
        if let Some(label) = block.label() {
            cross_ref.register_label(label.to_string());
            for target in block.extract_references() {
                cross_ref.add_reference(label.to_string(), target);
            }
        }

        Ok((block_label, change_reason))
    }

    /// Check if a label already exists in the document (recursive search through children)
    fn label_exists_in_blocks(blocks: &[Block], target_label: &str) -> bool {
        for block in blocks {
            // Check current block's label
            if let Some(label) = block.label() {
                if label == target_label {
                    return true;
                }
            }

            // Recursively check children
            if let Some(children) = block.children() {
                if Self::label_exists_in_blocks(children, target_label) {
                    return true;
                }
            }
        }
        false
    }

    /// Recursive insert BEFORE a block (searches nested children)
    fn insert_before_recursive(blocks: &mut Vec<Block>, anchor_label: &str, new_block: Block) -> DomainResult<()> {
        for (i, block) in blocks.iter().enumerate() {
            if block.label() == Some(anchor_label) {
                blocks.insert(i, new_block);
                return Ok(());
            }
        }

        // Search in children recursively
        for block in blocks.iter_mut() {
            if let Some(children) = block.children_mut() {
                if Self::insert_before_recursive(children, anchor_label, new_block.clone()).is_ok() {
                    return Ok(());
                }
            }
        }

        Err(DomainError::BlockNotFound {
            label: anchor_label.to_string(),
        })
    }

    /// Recursive insert AFTER a block (searches nested children)
    fn insert_after_recursive(blocks: &mut Vec<Block>, anchor_label: &str, new_block: Block) -> DomainResult<()> {
        for (i, block) in blocks.iter().enumerate() {
            if block.label() == Some(anchor_label) {
                blocks.insert(i + 1, new_block);
                return Ok(());
            }
        }

        // Search in children recursively
        for block in blocks.iter_mut() {
            if let Some(children) = block.children_mut() {
                if Self::insert_after_recursive(children, anchor_label, new_block.clone()).is_ok() {
                    return Ok(());
                }
            }
        }

        Err(DomainError::BlockNotFound {
            label: anchor_label.to_string(),
        })
    }

    /// Recursive insert INSIDE a parent block (adds to children array)
    fn insert_inside_recursive(blocks: &mut Vec<Block>, parent_label: &str, new_block: Block) -> DomainResult<()> {
        for block in blocks.iter_mut() {
            if block.label() == Some(parent_label) {
                // Found parent - add to its children
                if let Some(children) = block.children_mut() {
                    children.push(new_block);
                    return Ok(());
                } else {
                    return Err(DomainError::InvalidOperation {
                        reason: format!("Block {} does not support children", parent_label),
                    });
                }
            }

            // Search deeper
            if let Some(children) = block.children_mut() {
                if Self::insert_inside_recursive(children, parent_label, new_block.clone()).is_ok() {
                    return Ok(());
                }
            }
        }

        Err(DomainError::BlockNotFound {
            label: parent_label.to_string(),
        })
    }
}

impl DocumentOperations for RealIronBaseAdapter {
    fn insert_block(
        &mut self,
        document_id: &str,
        block: Block,
        options: InsertOptions,
    ) -> DomainResult<OperationResult> {
        let mut document = self.get_document(document_id)?;

        let (block_label, change_reason) = self.insert_block_into_document(&mut document, block, &options)?;

        // Update document metadata
        document.update_blocks_count();

        // Save document
        self.save_document(&document)?;

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels: vec![LabelChange {
                old_label: String::new(),
                new_label: block_label,
                reason: change_reason,
            }],
            warnings: Vec::new(),
        })
    }

    fn update_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        updates: HashMap<String, Value>,
    ) -> DomainResult<OperationResult> {
        let mut document = self.get_document(document_id)?;

        // Determine new label before borrowing
        let new_label = updates
            .get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Find block mutably and apply updates
        {
            let block = document
                .find_block_mut(block_label)
                .ok_or_else(|| DomainError::BlockNotFound {
                    label: block_label.to_string(),
                })?;

            // Apply updates (simplified - just update the label if provided)
            if let Some(ref label) = new_label {
                block.set_label(label.clone());
            }
        } // block borrow ends here

        // Update document metadata
        document.update_blocks_count();

        // Save document
        self.save_document(&document)?;

        let final_label = new_label.unwrap_or_else(|| block_label.to_string());

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels: vec![LabelChange {
                old_label: block_label.to_string(),
                new_label: final_label,
                reason: ChangeReason::Generated,
            }],
            warnings: Vec::new(),
        })
    }

    fn move_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        options: MoveOptions,
    ) -> DomainResult<OperationResult> {
        let mut document = self.get_document(document_id)?;

        // Step 1: Remove block from current location
        let block = document.remove_block(block_label)
            .ok_or_else(|| DomainError::BlockNotFound {
                label: block_label.to_string(),
            })?;

        // Step 2: Insert at new location
        // For simplicity, if no target parent specified, insert at document root
        if options.target_parent.is_none() {
            // Insert at document level based on position
            match options.position {
                InsertPosition::Start => {
                    document.docjll.insert(0, block);
                }
                InsertPosition::End => {
                    document.docjll.push(block);
                }
                InsertPosition::Before | InsertPosition::After | InsertPosition::Inside => {
                    // For now, fallback to End if no target specified
                    document.docjll.push(block);
                }
            }
        } else {
            // TODO: Insert into specific parent's children
            // This requires finding the parent and inserting at the right position
            document.docjll.push(block); // Fallback: add to root
        }

        // Update document metadata
        document.update_blocks_count();

        // Save document
        self.save_document(&document)?;

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels: vec![LabelChange {
                old_label: block_label.to_string(),
                new_label: block_label.to_string(), // Label unchanged for now
                reason: ChangeReason::Moved,
            }],
            warnings: if options.target_parent.is_some() {
                vec!["Move to specific parent not fully implemented - moved to document root".to_string()]
            } else {
                Vec::new()
            },
        })
    }

    fn delete_block(
        &mut self,
        document_id: &str,
        block_label: &str,
        options: DeleteOptions,
    ) -> DomainResult<OperationResult> {
        let mut document = self.get_document(document_id)?;

        // Check if block exists
        if document.find_block(block_label).is_none() {
            return Err(DomainError::BlockNotFound {
                label: block_label.to_string(),
            });
        }

        // Check for cross-references if requested
        if options.check_references && !options.force {
            let cross_ref = self.cross_ref.read();
            let referrers = cross_ref.get_referenced_by(block_label);
            if !referrers.is_empty() {
                return Err(DomainError::InvalidOperation {
                    reason: format!(
                        "Block {} is referenced by: {:?}. Use force=true to delete anyway.",
                        block_label,
                        referrers
                    ),
                });
            }
        }

        // Perform actual deletion
        let removed_blocks = if options.cascade {
            // Remove block and all its children
            document.remove_block_cascade(block_label)
        } else {
            // Remove only the block (children become orphaned or moved up)
            document.remove_block(block_label).map(|b| vec![b])
        };

        if removed_blocks.is_none() {
            return Err(DomainError::BlockNotFound {
                label: block_label.to_string(),
            });
        }

        let removed = removed_blocks.unwrap();
        let mut affected_labels = Vec::new();

        // Update cross-references for all removed blocks
        {
            let mut cross_ref = self.cross_ref.write();
            for block in &removed {
                if let Some(label) = block.label() {
                    cross_ref.remove_label(label);
                    affected_labels.push(LabelChange {
                        old_label: label.to_string(),
                        new_label: String::new(),
                        reason: ChangeReason::Generated,
                    });
                }
            }
        }

        // Update document metadata
        document.update_blocks_count();

        // Save document
        self.save_document(&document)?;

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels,
            warnings: Vec::new(),
        })
    }

    fn get_outline(
        &self,
        document_id: &str,
        max_depth: Option<usize>,
    ) -> DomainResult<Vec<OutlineItem>> {
        let document = self.get_document(document_id)?;
        let mut outline = Vec::new();

        fn extract_headings(
            blocks: &[Block],
            depth: usize,
            max_depth: Option<usize>,
        ) -> Vec<OutlineItem> {
            let mut items = Vec::new();

            for block in blocks {
                if let Block::Heading(h) = block {
                    let title = crate::domain::block::inline_to_plain_text(&h.content);
                    let children = if max_depth.map_or(true, |max| depth < max) {
                        h.children
                            .as_ref()
                            .map(|c| extract_headings(c, depth + 1, max_depth))
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    items.push(OutlineItem {
                        level: h.level.unwrap_or(1),  // Default to level 1 if not specified
                        label: h.label.clone().unwrap_or_default(),
                        title,
                        children,
                    });
                }
                // Note: We don't need to process block.children() here because
                // heading children are already processed above (lines 739-746)
            }

            items
        }

        outline.extend(extract_headings(&document.docjll, 0, max_depth));
        Ok(outline)
    }

    fn search_blocks(
        &self,
        document_id: &str,
        query: SearchQuery,
    ) -> DomainResult<Vec<SearchResult>> {
        let document = self.get_document(document_id)?;
        let mut results = Vec::new();

        fn search_recursive(
            blocks: &[Block],
            query: &SearchQuery,
            path: &mut Vec<String>,
            results: &mut Vec<SearchResult>,
        ) {
            for block in blocks {
                let mut matches = true;

                // Filter by block type
                if let Some(ref target_type) = query.block_type {
                    matches = matches && (&block.block_type() == target_type);
                }

                // Filter by has_label
                if let Some(has_label) = query.has_label {
                    matches = matches && (block.label().is_some() == has_label);
                }

                // Filter by exact label match
                if let Some(ref exact_label) = query.label {
                    matches = matches && block.label().map(|l| l == exact_label).unwrap_or(false);
                }

                // Filter by label prefix
                if let Some(ref prefix) = query.label_prefix {
                    matches = matches && block.label().map(|l| l.starts_with(prefix)).unwrap_or(false);
                }

                // Filter by level (for headings)
                if let Some(target_level) = query.level {
                    matches = matches && match block {
                        Block::Heading(h) => h.level == Some(target_level),
                        _ => false, // Non-heading blocks don't match level filter
                    };
                }

                if matches {
                    if let Some(label) = block.label() {
                        results.push(SearchResult {
                            label: label.to_string(),
                            block: block.clone(),
                            path: path.clone(),
                            score: 1.0,
                        });
                    }
                }

                if let Some(children) = block.children() {
                    if let Some(label) = block.label() {
                        path.push(label.to_string());
                    }
                    search_recursive(children, query, path, results);
                    if block.label().is_some() {
                        path.pop();
                    }
                }
            }
        }

        let mut path = Vec::new();
        search_recursive(&document.docjll, &query, &mut path, &mut results);

        Ok(results)
    }

    fn validate_references(&self, document_id: &str) -> DomainResult<ValidationResult> {
        let document = self.get_document(document_id)?;

        let mut validator = ReferenceValidator::new();
        validator.build_from_blocks(&document.docjll);

        let broken_refs = validator.validate();

        let mut result = ValidationResult::success();
        for broken_ref in broken_refs {
            result.add_error(crate::domain::validation::ValidationError {
                block_label: Some(broken_ref.source),
                field: Some("reference".to_string()),
                message: broken_ref.error,
                error_type: crate::domain::validation::ErrorType::ReferenceError,
            });
        }

        Ok(result)
    }

    fn validate_schema(&self, document_id: &str) -> DomainResult<ValidationResult> {
        let document = self.get_document(document_id)?;

        let validator = self.schema_validator.read();
        Ok(validator.validate_document(&document))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn seed_test_document(db_path: &PathBuf) {
        let db = DatabaseCore::<StorageEngine>::open(db_path).unwrap();
        let collection = db.collection("documents").unwrap();
        let doc = json!({
            "id": "mk_manual_v1",
            "metadata": {
                "title": "Manual",
                "version": "1.0"
            },
            "docjll": [
                {
                    "type": "paragraph",
                    "label": "para:1",
                    "content": [
                        {"type": "text", "content": "Original paragraph"}
                    ]
                }
            ]
        });
        let map = doc
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<HashMap<_, _>>();
        collection.insert_one(map).unwrap();
    }

    #[test]
    fn insert_block_supports_numeric_ids() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.mlite");
        seed_test_document(&db_path);

        let mut adapter = RealIronBaseAdapter::new(db_path.clone(), "documents".to_string()).unwrap();
        let block: Block = serde_json::from_value(json!({
            "type": "paragraph",
            "content": [{"type": "text", "content": "Inserted via test"}]
        }))
        .unwrap();

        let result = adapter
            .insert_block("1", block, InsertOptions::default())
            .unwrap();
        assert!(result.success);

        let updated = adapter.get_document("1").unwrap();
        assert!(updated.docjll.len() >= 2);
    }
}
