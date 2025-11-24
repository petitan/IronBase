// Integration tests for MCP DOCJL Server

use mcp_docjl::{
    Block, Document, DocumentOperations, InsertOptions, InsertPosition, IronBaseAdapter,
    SearchQuery,
};
use std::collections::HashMap;
use tempfile::tempdir;

/// Helper to create test document
fn create_test_document(id: &str) -> Document {
    use mcp_docjl::domain::block::{Heading, InlineContent, Paragraph};
    use mcp_docjl::domain::document::DocumentMetadata;

    Document {
        db_id: None,
        id: Some(id.to_string()),
        metadata: DocumentMetadata {
            title: "Test Document".to_string(),
            version: "1.0".to_string(),
            author: Some("Test Author".to_string()),
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            modified_at: None,
            blocks_count: Some(0),
            tags: None,
            custom_fields: None,
        },
        docjll: vec![
            Block::Heading(Heading {
                level: Some(1),
                content: vec![InlineContent::Text {
                    content: "Introduction".to_string(),
                }],
                label: Some("sec:1".to_string()),
                children: Some(vec![Block::Paragraph(Paragraph {
                    content: vec![InlineContent::Text {
                        content: "This is the introduction.".to_string(),
                    }],
                    label: Some("para:1.1".to_string()),
                    compliance_note: None,
                })]),
            }),
            Block::Heading(Heading {
                level: Some(1),
                content: vec![InlineContent::Text {
                    content: "Methods".to_string(),
                }],
                label: Some("sec:2".to_string()),
                children: None,
            }),
        ],
    }
}

#[test]
fn test_adapter_initialization() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string());
    assert!(adapter.is_ok());
}

#[test]
fn test_insert_block() {
    use mcp_docjl::domain::block::{InlineContent, Paragraph};

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create and save test document
    let doc = create_test_document("test_doc_1");
    adapter.insert_document_for_test(doc);

    // Insert a new paragraph
    let new_block = Block::Paragraph(Paragraph {
        content: vec![InlineContent::Text {
            content: "New paragraph content".to_string(),
        }],
        label: None,
        compliance_note: None,
    });

    let result = adapter
        .insert_block(
            "test_doc_1",
            new_block,
            InsertOptions {
                parent_label: None,
                position: InsertPosition::End,
                anchor_label: None,
                auto_label: true,
                validate: true,
            },
        )
        .unwrap();

    assert!(result.success);
    assert_eq!(result.affected_labels.len(), 1);

    // Verify document was updated
    let updated_doc = adapter.get_document("test_doc_1").unwrap();
    assert_eq!(updated_doc.docjll.len(), 3); // Original 2 + 1 new
}

#[test]
fn test_get_outline() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create and save test document
    let doc = create_test_document("test_doc_2");
    adapter.insert_document_for_test(doc);

    // Get outline
    let outline = adapter.get_outline("test_doc_2", None).unwrap();

    assert_eq!(outline.len(), 2); // Two top-level headings
    assert_eq!(outline[0].label, "sec:1");
    assert_eq!(outline[0].title, "Introduction");
    assert_eq!(outline[1].label, "sec:2");
    assert_eq!(outline[1].title, "Methods");
}

#[test]
fn test_search_blocks() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create and save test document
    let doc = create_test_document("test_doc_3");
    adapter.insert_document_for_test(doc);

    // Search for headings
    let query = SearchQuery {
        block_type: Some(mcp_docjl::BlockType::Heading),
        content_contains: None,
        has_label: Some(true),
        has_compliance_note: None,
        label: None,
        label_prefix: None,
    };

    let results = adapter.search_blocks("test_doc_3", query).unwrap();

    assert_eq!(results.len(), 2); // Two headings in the document

    // Verify label filter returns a single match
    let label_query = SearchQuery {
        block_type: None,
        content_contains: None,
        has_label: Some(true),
        has_compliance_note: None,
        label: Some("sec:1".to_string()),
        label_prefix: None,
    };
    let filtered = adapter.search_blocks("test_doc_3", label_query).unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].label, "sec:1");
}

#[test]
fn test_validate_schema() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create and save test document
    let doc = create_test_document("test_doc_4");
    adapter.insert_document_for_test(doc);

    // Validate schema
    let result = adapter.validate_schema("test_doc_4").unwrap();

    assert!(result.valid);
    assert!(result.errors.is_empty());
}

#[test]
fn test_validate_references() {
    use mcp_docjl::domain::block::{InlineContent, Paragraph};

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create document with cross-reference
    let mut doc = create_test_document("test_doc_5");

    // Add paragraph with reference
    doc.docjll.push(Block::Paragraph(Paragraph {
        content: vec![
            InlineContent::Text {
                content: "See section ".to_string(),
            },
            InlineContent::Ref {
                target: "sec:1".to_string(),
            },
        ],
        label: Some("para:3".to_string()),
        compliance_note: None,
    }));

    adapter.insert_document_for_test(doc);

    // Validate references
    let result = adapter.validate_references("test_doc_5").unwrap();

    // Should be valid since sec:1 exists
    assert!(result.valid);
    assert!(result.errors.is_empty());
}

#[test]
fn test_broken_reference_detection() {
    use mcp_docjl::domain::block::{InlineContent, Paragraph};

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create document with broken reference
    let mut doc = create_test_document("test_doc_6");

    // Add paragraph with broken reference
    doc.docjll.push(Block::Paragraph(Paragraph {
        content: vec![
            InlineContent::Text {
                content: "See section ".to_string(),
            },
            InlineContent::Ref {
                target: "sec:99".to_string(), // Non-existent
            },
        ],
        label: Some("para:4".to_string()),
        compliance_note: None,
    }));

    adapter.insert_document_for_test(doc);

    // Validate references
    let result = adapter.validate_references("test_doc_6").unwrap();

    // Should be invalid
    assert!(!result.valid);
    assert!(!result.errors.is_empty());
    assert_eq!(result.errors[0].error_type, mcp_docjl::domain::validation::ErrorType::ReferenceError);
}

#[test]
fn test_update_block() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create and save test document
    let doc = create_test_document("test_doc_7");
    adapter.insert_document_for_test(doc);

    // Update a block
    let mut updates = HashMap::new();
    updates.insert("label".to_string(), serde_json::json!("sec:1_updated"));

    let result = adapter.update_block("test_doc_7", "sec:1", updates).unwrap();

    assert!(result.success);
}

#[test]
fn test_label_generator() {
    use mcp_docjl::LabelGenerator;

    let mut gen = LabelGenerator::new();

    // Register existing labels
    gen.register("para:1").unwrap();
    gen.register("para:2").unwrap();
    gen.register("sec:1").unwrap();

    // Generate new labels
    let para_label = gen.generate("para");
    assert_eq!(para_label, "para:3");

    let sec_label = gen.generate("sec");
    assert_eq!(sec_label, "sec:2");

    // Test uniqueness
    assert!(gen.exists("para:1"));
    assert!(gen.exists("para:3"));
    assert!(!gen.exists("para:99"));
}

#[test]
fn test_list_documents() {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let mut adapter = IronBaseAdapter::new(db_path, "documents".to_string()).unwrap();

    // Create multiple documents
    let doc1 = create_test_document("doc_1");
    let doc2 = create_test_document("doc_2");

    adapter.insert_document_for_test(doc1);
    adapter.insert_document_for_test(doc2);

    // List all documents
    let documents = adapter.list_documents().unwrap();

    assert_eq!(documents.len(), 2);
}

#[test]
fn test_invalid_block_validation() {
    use mcp_docjl::domain::block::Heading;
    use mcp_docjl::SchemaValidator;

    let validator = SchemaValidator::default();

    // Create heading with invalid level
    let invalid_heading = Block::Heading(Heading {
        level: Some(10), // Invalid - should be 1-6
        content: vec![],
        label: None,
        children: None,
    });

    let result = validator.validate_block(&invalid_heading);

    assert!(!result.valid);
    assert!(!result.errors.is_empty());
}

#[test]
fn test_concurrent_inserts() {
    use std::sync::Arc;
    use std::thread;

    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("test.mlite");

    let adapter = Arc::new(parking_lot::RwLock::new(
        IronBaseAdapter::new(db_path, "documents".to_string()).unwrap(),
    ));

    // Create test document
    let doc = create_test_document("concurrent_doc");
    adapter.write().insert_document_for_test(doc);

    let mut handles = vec![];

    // Spawn multiple threads to insert blocks
    for i in 0..5 {
        let adapter_clone = Arc::clone(&adapter);
        let handle = thread::spawn(move || {
            use mcp_docjl::domain::block::{InlineContent, Paragraph};

            let block = Block::Paragraph(Paragraph {
                content: vec![InlineContent::Text {
                    content: format!("Concurrent paragraph {}", i),
                }],
                label: None,
                compliance_note: None,
            });

            adapter_clone
                .write()
                .insert_block("concurrent_doc", block, InsertOptions::default())
        });
        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        assert!(handle.join().unwrap().is_ok());
    }

    // Verify all blocks were inserted
    let final_doc = adapter.read().get_document("concurrent_doc").unwrap();
    assert_eq!(final_doc.docjll.len(), 7); // 2 original + 5 new
}
