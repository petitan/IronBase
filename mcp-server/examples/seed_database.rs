/// Seed the database with test documents
///
/// This creates sample DOCJL documents for testing the MCP server.

use mcp_docjl::{
    Block, Document, IronBaseAdapter,
    domain::{DocumentMetadata, block::{Heading, Paragraph, InlineContent}},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🌱 Seeding database with test documents...");

    // Create adapter
    let mut adapter = IronBaseAdapter::new(
        "./docjl_storage.mlite".into(),
        "documents".to_string()
    )?;

    // Create test document 1
    let doc1 = Document {
        id: "test_doc_1".to_string(),
        metadata: DocumentMetadata {
            title: "Test Document 1".to_string(),
            version: "1.0".to_string(),
            author: Some("Claude".to_string()),
            created_at: Some("2025-11-21T21:00:00Z".to_string()),
            modified_at: Some("2025-11-21T21:00:00Z".to_string()),
            blocks_count: Some(3),
            tags: Some(vec!["test".to_string(), "demo".to_string()]),
            custom_fields: None,
        },
        docjll: vec![
            Block::Heading(Heading {
                level: 1,
                label: Some("sec:1".to_string()),
                content: vec![InlineContent::Text {
                    content: "Introduction".to_string(),
                }],
                children: None,
            }),
            Block::Paragraph(Paragraph {
                content: vec![
                    InlineContent::Text {
                        content: "This is a ".to_string(),
                    },
                    InlineContent::Bold {
                        content: "test document".to_string(),
                    },
                    InlineContent::Text {
                        content: " created for MCP testing.".to_string(),
                    },
                ],
                label: Some("para:1".to_string()),
                children: None,
            }),
            Block::Heading(Heading {
                level: 2,
                label: Some("sec:2".to_string()),
                content: vec![InlineContent::Text {
                    content: "Features".to_string(),
                }],
                children: None,
            }),
        ],
    };

    adapter.insert_document_for_test(doc1);
    println!("✅ Created: test_doc_1");

    // Create test document 2
    let doc2 = Document {
        id: "test_doc_2".to_string(),
        metadata: DocumentMetadata {
            title: "Requirements Specification".to_string(),
            version: "2.0".to_string(),
            author: Some("AI Assistant".to_string()),
            created_at: Some("2025-11-21T21:00:00Z".to_string()),
            modified_at: Some("2025-11-21T21:00:00Z".to_string()),
            blocks_count: Some(4),
            tags: Some(vec!["requirements".to_string(), "spec".to_string()]),
            custom_fields: None,
        },
        docjll: vec![
            Block::Heading(Heading {
                level: 1,
                label: Some("sec:1".to_string()),
                content: vec![InlineContent::Text {
                    content: "Functional Requirements".to_string(),
                }],
                children: None,
            }),
            Block::Paragraph(Paragraph {
                content: vec![
                    InlineContent::Text {
                        content: "The system shall support ".to_string(),
                    },
                    InlineContent::Italic {
                        content: "real-time collaboration".to_string(),
                    },
                    InlineContent::Text {
                        content: " on documents.".to_string(),
                    },
                ],
                label: Some("req:1".to_string()),
                children: None,
            }),
            Block::Paragraph(Paragraph {
                content: vec![InlineContent::Text {
                    content: "Cross-reference example: see requirement ".to_string(),
                }, InlineContent::Ref {
                    target: "req:1".to_string(),
                }],
                label: Some("req:2".to_string()),
                children: None,
            }),
            Block::Heading(Heading {
                level: 2,
                label: Some("sec:2".to_string()),
                content: vec![InlineContent::Text {
                    content: "Non-Functional Requirements".to_string(),
                }],
                children: None,
            }),
        ],
    };

    adapter.insert_document_for_test(doc2);
    println!("✅ Created: test_doc_2");

    println!("\n🎉 Database seeded successfully!");
    println!("You can now test the MCP server with these documents.");

    Ok(())
}
