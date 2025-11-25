// IronBase storage adapter for DOCJL documents

use crate::domain::{
    Block, CrossReference, DeleteOptions, Document, DocumentOperations, DomainError,
    DomainResult, InsertOptions, InsertPosition, LabelGenerator,
    MoveOptions, OperationResult, OutlineItem, ReferenceValidator, SchemaValidator,
    SearchQuery, SearchResult, ValidationResult, LabelChange, ChangeReason,
};
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// IronBase adapter for DOCJL documents
pub struct IronBaseAdapter {
    // TODO: Replace with actual IronBase once integrated
    // db: Arc<RwLock<IronBase>>,
    #[allow(dead_code)]
    collection_name: String,
    label_generator: Arc<RwLock<LabelGenerator>>,
    cross_ref: Arc<RwLock<CrossReference>>,
    schema_validator: Arc<RwLock<SchemaValidator>>,
    // Temporary in-memory storage for development
    documents: Arc<RwLock<HashMap<String, Document>>>,
}

impl IronBaseAdapter {
    /// Create a new adapter
    pub fn new(_path: PathBuf, collection_name: String) -> DomainResult<Self> {
        // TODO: Open IronBase database
        // let db = IronBase::open(path).map_err(|e| DomainError::StorageError {
        //     message: e.to_string(),
        // })?;

        Ok(Self {
            collection_name,
            label_generator: Arc::new(RwLock::new(LabelGenerator::new())),
            cross_ref: Arc::new(RwLock::new(CrossReference::new())),
            schema_validator: Arc::new(RwLock::new(SchemaValidator::default())),
            documents: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Load all documents and build indexes
    pub fn initialize(&self) -> DomainResult<()> {
        // TODO: Load from IronBase
        // let collection = self.db.read().collection(&self.collection_name)?;
        // let docs = collection.find(&serde_json::json!({}))?;

        // For now, initialize with empty state
        let mut label_gen = self.label_generator.write();
        let mut cross_ref = self.cross_ref.write();

        // Load documents and extract labels/references
        let documents = self.documents.read();
        for doc in documents.values() {
            for label in doc.collect_labels() {
                label_gen.register(&label)?;
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

    /// Create a new document
    pub fn create_document(&self, document: Document) -> DomainResult<String> {
        let mut documents = self.documents.write();
        let document_id = document.id.clone().unwrap_or_else(|| format!("doc_{}", documents.len() + 1));

        // Store by document ID
        documents.insert(document_id.clone(), document);

        Ok(document_id)
    }

    /// Get a document by ID (supports both _id and semantic id field lookup)
    pub fn get_document(&self, document_id: &str) -> DomainResult<Document> {
        let documents = self.documents.read();

        // First try direct lookup by HashMap key (_id)
        if let Some(doc) = documents.get(document_id) {
            return Ok(doc.clone());
        }

        // If not found, search by semantic id or db_id
        for doc in documents.values() {
            if doc.matches_identifier(document_id) {
                return Ok(doc.clone());
            }
        }

        // Not found by either method
        Err(DomainError::StorageError {
            message: format!("Document not found: {}", document_id),
        })
    }

    /// Save a document
    fn save_document(&self, document: &Document) -> DomainResult<()> {
        // TODO: Save to IronBase
        // let collection = self.db.write().collection(&self.collection_name)?;
        // collection.update_one(
        //     &serde_json::json!({"_id": document.id}),
        //     &serde_json::to_value(document)?,
        // )?;

        // Temporary in-memory storage
        let key = document.identifier().ok_or_else(|| DomainError::StorageError {
            message: "Document missing identifier".to_string(),
        })?;

        let mut documents = self.documents.write();
        documents.insert(key, document.clone());
        Ok(())
    }

    /// List all documents
    pub fn list_documents(&self) -> DomainResult<Vec<Document>> {
        // TODO: Query from IronBase
        let documents = self.documents.read();
        Ok(documents.values().cloned().collect())
    }

    /// Find a block in a document
    #[allow(dead_code)]
    fn find_block_in_document<'a>(
        &self,
        document: &'a Document,
        label: &str,
    ) -> DomainResult<&'a Block> {
        document
            .find_block(label)
            .ok_or_else(|| DomainError::BlockNotFound {
                label: label.to_string(),
            })
    }

    /// Find a block mutably
    fn find_block_in_document_mut<'a>(
        &self,
        document: &'a mut Document,
        label: &str,
    ) -> DomainResult<&'a mut Block> {
        document
            .find_block_mut(label)
            .ok_or_else(|| DomainError::BlockNotFound {
                label: label.to_string(),
            })
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

        // Find insertion point
        match options.position {
            InsertPosition::Start => {
                // Add to beginning of document
                document.docjll.insert(0, block.clone());
            }
            InsertPosition::End => {
                // Add to end of document
                document.docjll.push(block.clone());
            }
            InsertPosition::Before | InsertPosition::After => {
                if let Some(ref anchor) = options.anchor_label {
                    self.insert_relative_to_anchor(document, block.clone(), anchor, options.position)?;
                } else {
                    return Err(DomainError::InvalidOperation {
                        reason: "Before/After position requires anchor_label".to_string(),
                    });
                }
            }
            InsertPosition::Inside => {
                if let Some(ref parent_label) = options.parent_label {
                    let parent = self.find_block_in_document_mut(document, parent_label)?;
                    if let Some(children) = parent.children_mut() {
                        children.insert(0, block.clone());
                    } else {
                        return Err(DomainError::InvalidOperation {
                            reason: format!(
                                "Block {} cannot have children",
                                parent_label
                            ),
                        });
                    }
                } else {
                    return Err(DomainError::InvalidOperation {
                        reason: "Inside position requires parent_label".to_string(),
                    });
                }
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

    /// Insert block relative to an anchor
    fn insert_relative_to_anchor(
        &self,
        document: &mut Document,
        block: Block,
        anchor_label: &str,
        position: InsertPosition,
    ) -> DomainResult<()> {
        // Find anchor position recursively
        fn find_and_insert(
            blocks: &mut Vec<Block>,
            block: Block,
            anchor_label: &str,
            position: InsertPosition,
        ) -> Result<bool, DomainError> {
            for (i, existing) in blocks.iter_mut().enumerate() {
                if existing.label() == Some(anchor_label) {
                    let insert_pos = match position {
                        InsertPosition::Before => i,
                        InsertPosition::After => i + 1,
                        _ => i,
                    };
                    blocks.insert(insert_pos, block);
                    return Ok(true);
                }

                // Check children
                if let Some(children) = existing.children_mut() {
                    if find_and_insert(children, block.clone(), anchor_label, position)? {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }

        let found = find_and_insert(&mut document.docjll, block, anchor_label, position)?;
        if !found {
            return Err(DomainError::BlockNotFound {
                label: anchor_label.to_string(),
            });
        }

        Ok(())
    }

    /// Test helper: Insert a document directly (for testing only)
    ///
    /// This method is public for integration testing purposes.
    /// Do not use in production code!
    pub fn insert_document_for_test(&mut self, document: Document) {
        if let Some(key) = document.identifier() {
            self.documents.write().insert(key, document);
        }
    }
}

impl DocumentOperations for IronBaseAdapter {
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

        // Find and update block
        let block = self.find_block_in_document_mut(&mut document, block_label)?;

        // Apply updates based on block type
        // TODO: Implement comprehensive update logic for each field
        for (key, value) in updates {
            match key.as_str() {
                "content" => {
                    // Update content (needs type-specific handling)
                    // This is simplified - real implementation needs to deserialize properly
                }
                "label" => {
                    if let Some(new_label) = value.as_str() {
                        // Update cross-references
                        let mut cross_ref = self.cross_ref.write();
                        cross_ref.update_label(block_label, new_label.to_string());

                        block.set_label(new_label.to_string());
                    }
                }
                _ => {
                    // Handle other fields
                }
            }
        }

        // Save document
        self.save_document(&document)?;

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels: vec![],
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
        if options.target_parent.is_none() {
            // Insert at document level
            match options.position {
                InsertPosition::Start => {
                    document.docjll.insert(0, block);
                }
                InsertPosition::End => {
                    document.docjll.push(block);
                }
                InsertPosition::Before | InsertPosition::After | InsertPosition::Inside => {
                    document.docjll.push(block); // Fallback to End
                }
            }
        } else {
            // TODO: Insert into specific parent
            document.docjll.push(block); // Fallback: add to root
        }

        // Save document
        self.save_document(&document)?;

        Ok(OperationResult {
            success: true,
            audit_id: self.generate_audit_id(),
            affected_labels: vec![LabelChange {
                old_label: block_label.to_string(),
                new_label: block_label.to_string(),
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

        // Check references if requested
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
            // Remove only the block
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
                // heading children are already processed above (lines 564-571)
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

                // Type filter
                if let Some(ref target_type) = query.block_type {
                    matches = matches && (&block.block_type() == target_type);
                }

                // Content filter (simplified)
                if let Some(ref _search_text) = query.content_contains {
                    // TODO: Extract text content from block and search
                    matches = matches && true; // Placeholder
                }

                // Label filter
                if let Some(has_label) = query.has_label {
                    matches = matches && (block.label().is_some() == has_label);
                }

                // Exact label match
                if let Some(ref exact_label) = query.label {
                    matches =
                        matches && block.label().map(|l| l == exact_label).unwrap_or(false);
                }

                // Label prefix match
                if let Some(ref prefix) = query.label_prefix {
                    matches =
                        matches && block.label().map(|l| l.starts_with(prefix)).unwrap_or(false);
                }

                // Level filter (for headings)
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
                            score: 1.0, // TODO: Implement relevance scoring
                        });
                    }
                }

                // Search children
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
    use crate::domain::block::{InlineContent, Paragraph};
    use tempfile::tempdir;

    #[test]
    fn test_insert_block() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.mlite");

        let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

        // Create test document
        let doc = Document {
            db_id: None,
            id: Some("test_doc".to_string()),
            metadata: crate::domain::document::DocumentMetadata {
                title: "Test".to_string(),
                version: "1.0".to_string(),
                author: None,
                created_at: None,
                modified_at: None,
                blocks_count: None,
                tags: None,
                custom_fields: None,
            },
            docjll: vec![],
        };

        adapter.insert_document_for_test(doc);

        // Insert block
        let block = Block::Paragraph(Paragraph {
            content: vec![InlineContent::Text {
                content: "Test paragraph".to_string(),
            }],
            label: None,
            compliance_note: None,
        });

        let result = adapter
            .insert_block("test_doc", block, InsertOptions::default())
            .unwrap();

        assert!(result.success);
        assert!(!result.affected_labels.is_empty());

        // Verify document was updated
        let updated_doc = adapter.get_document("test_doc").unwrap();
        assert_eq!(updated_doc.docjll.len(), 1);
    }
}
