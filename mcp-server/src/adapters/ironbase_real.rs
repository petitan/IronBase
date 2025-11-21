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
    /// Create a new adapter with real IronBase
    pub fn new(path: PathBuf, collection_name: String) -> DomainResult<Self> {
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
        let db = self.db.read();
        let collection = db.collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        // Find all documents
        let docs_json = collection.find(&serde_json::json!({}))
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to find documents: {}", e),
            })?;

        let mut label_gen = self.label_generator.write();
        let mut cross_ref = self.cross_ref.write();

        // Process each document
        for mut doc_json in docs_json {
            // Convert integer _id to string if needed (IronBase uses integer IDs)
            if let Some(id) = doc_json.get("_id") {
                if let Some(id_num) = id.as_i64() {
                    doc_json["_id"] = serde_json::json!(id_num.to_string());
                }
            }

            // Parse as DOCJL Document
            let doc: Document = serde_json::from_value(doc_json)
                .map_err(|e| DomainError::StorageError {
                    message: format!("Failed to parse document: {}", e),
                })?;

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

    /// Get a document by ID from IronBase
    pub fn get_document(&self, document_id: &str) -> DomainResult<Document> {
        let db = self.db.read();
        let collection = db.collection(&self.collection_name)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to get collection: {}", e),
            })?;

        // Try to parse document_id as integer for IronBase query
        let id_value = document_id.parse::<i64>()
            .map(|n| serde_json::json!(n))
            .unwrap_or_else(|_| serde_json::json!(document_id));

        let query = serde_json::json!({"_id": id_value});
        let docs = collection.find(&query)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to find document: {}", e),
            })?;

        if docs.is_empty() {
            return Err(DomainError::StorageError {
                message: format!("Document not found: {}", document_id),
            });
        }

        let mut doc_json = docs[0].clone();

        // Convert integer _id to string if needed (IronBase uses integer IDs)
        if let Some(id) = doc_json.get("_id") {
            if let Some(id_num) = id.as_i64() {
                doc_json["_id"] = serde_json::json!(id_num.to_string());
            }
        }

        let doc: Document = serde_json::from_value(doc_json)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to parse document: {}", e),
            })?;

        Ok(doc)
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

        // Convert string _id to integer for IronBase
        let id_int = document.id.parse::<i64>()
            .map_err(|_| DomainError::StorageError {
                message: format!("Invalid document ID format: {}", document.id),
            })?;

        doc_value["_id"] = serde_json::json!(id_int);

        // Use IronBase update_one with $set operator to replace document
        let query = serde_json::json!({"_id": id_int});
        let update = serde_json::json!({"$set": doc_value});

        collection.update_one(&query, &update)
            .map_err(|e| DomainError::StorageError {
                message: format!("Failed to update document: {}", e),
            })?;

        Ok(())
    }

    /// List all documents from IronBase
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
        for mut doc_json in docs_json {
            // Convert integer _id to string if needed (IronBase uses integer IDs)
            if let Some(id) = doc_json.get("_id") {
                if let Some(id_num) = id.as_i64() {
                    doc_json["_id"] = serde_json::json!(id_num.to_string());
                }
            }

            let doc: Document = serde_json::from_value(doc_json)
                .map_err(|e| DomainError::StorageError {
                    message: format!("Failed to parse document: {}", e),
                })?;
            documents.push(doc);
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

    /// Insert block into document at specified position
    fn insert_block_into_document(
        &self,
        document: &mut Document,
        mut block: Block,
        options: &InsertOptions,
    ) -> DomainResult<String> {
        // Auto-generate label if needed
        let block_label = if block.label().is_none() && options.auto_label {
            let prefix = crate::domain::label::default_prefix_for_type(
                &format!("{:?}", block.block_type()).to_lowercase(),
            );
            let label = self.label_generator.write().generate(prefix);
            block.set_label(label.clone());
            label
        } else {
            block.label().unwrap_or("unlabeled").to_string()
        };

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

        // Insert based on position
        match options.position {
            InsertPosition::End => {
                document.docjll.push(block.clone());
            }
            _ => {
                // TODO: Implement other positions
                return Err(DomainError::InvalidOperation {
                    reason: "Only 'end' position is currently supported".to_string(),
                });
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

        Ok(block_label)
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

        let block_label = self.insert_block_into_document(&mut document, block, &options)?;

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
                reason: ChangeReason::Generated,
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
                        level: h.level,
                        label: h.label.clone().unwrap_or_default(),
                        title,
                        children,
                    });
                }

                if let Some(children) = block.children() {
                    if max_depth.map_or(true, |max| depth < max) {
                        items.extend(extract_headings(children, depth + 1, max_depth));
                    }
                }
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

                if let Some(ref target_type) = query.block_type {
                    if &block.block_type() != target_type {
                        matches = false;
                    }
                }

                if let Some(has_label) = query.has_label {
                    matches = matches && (block.label().is_some() == has_label);
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
