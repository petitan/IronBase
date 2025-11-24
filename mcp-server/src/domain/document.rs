// DOCJL document structure and metadata

use super::Block;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A complete DOCJL document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    #[serde(alias = "_id", skip_serializing)]
    pub db_id: Option<Value>,

    #[serde(default)]
    pub id: Option<String>,

    pub metadata: DocumentMetadata,

    #[serde(alias = "blocks")]
    pub docjll: Vec<Block>,
}

/// Document metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: String,
    pub version: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_count: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_fields: Option<HashMap<String, serde_json::Value>>,
}

impl Document {
    /// Count total blocks (including nested)
    pub fn count_blocks(&self) -> usize {
        count_blocks_recursive(&self.docjll)
    }

    /// Preferred identifier (semantic id or fallback to db_id)
    pub fn identifier(&self) -> Option<String> {
        self.id
            .as_ref()
            .map(|s| s.to_string())
            .or_else(|| self.db_id_as_string())
    }

    /// True if given identifier matches semantic id or underlying db_id
    pub fn matches_identifier(&self, candidate: &str) -> bool {
        if self.id.as_deref() == Some(candidate) {
            return true;
        }
        match self.db_id.as_ref() {
            Some(Value::String(s)) => s == candidate,
            Some(Value::Number(n)) => n.to_string() == candidate,
            Some(Value::Bool(b)) => b.to_string() == candidate,
            Some(other) if other.is_null() => false,
            Some(other) => other.to_string() == candidate,
            None => false,
        }
    }

    /// Convert stored db_id into a printable string if available
    pub fn db_id_as_string(&self) -> Option<String> {
        self.db_id.as_ref().and_then(|value| match value {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Null => None,
            other => Some(other.to_string()),
        })
    }

    /// Find a block by label
    pub fn find_block(&self, label: &str) -> Option<&Block> {
        find_block_recursive(&self.docjll, label)
    }

    /// Find a block by label (mutable)
    pub fn find_block_mut(&mut self, label: &str) -> Option<&mut Block> {
        find_block_recursive_mut(&mut self.docjll, label)
    }

    /// Collect all labels in the document
    pub fn collect_labels(&self) -> Vec<String> {
        collect_labels_recursive(&self.docjll)
    }

    /// Update blocks count in metadata
    pub fn update_blocks_count(&mut self) {
        self.metadata.blocks_count = Some(self.count_blocks());
    }

    /// Remove a block from the document (returns the removed block if found)
    pub fn remove_block(&mut self, label: &str) -> Option<Block> {
        remove_block_recursive(&mut self.docjll, label)
    }

    /// Remove a block from the document with cascade (removes children too)
    pub fn remove_block_cascade(&mut self, label: &str) -> Option<Vec<Block>> {
        // Remove the block and get it
        if let Some(block) = self.remove_block(label) {
            // Collect all removed blocks (parent + children)
            let mut removed = vec![block.clone()];

            // Add children recursively
            fn collect_children(block: &Block, collected: &mut Vec<Block>) {
                if let Some(children) = block.children() {
                    for child in children {
                        collected.push(child.clone());
                        collect_children(child, collected);
                    }
                }
            }
            collect_children(&block, &mut removed);

            Some(removed)
        } else {
            None
        }
    }
}

fn count_blocks_recursive(blocks: &[Block]) -> usize {
    let mut count = blocks.len();
    for block in blocks {
        if let Some(children) = block.children() {
            count += count_blocks_recursive(children);
        }
    }
    count
}

fn find_block_recursive<'a>(blocks: &'a [Block], label: &str) -> Option<&'a Block> {
    for block in blocks {
        if block.label() == Some(label) {
            return Some(block);
        }
        if let Some(children) = block.children() {
            if let Some(found) = find_block_recursive(children, label) {
                return Some(found);
            }
        }
    }
    None
}

fn find_block_recursive_mut<'a>(blocks: &'a mut [Block], label: &str) -> Option<&'a mut Block> {
    for block in blocks {
        if block.label() == Some(label) {
            return Some(block);
        }
        // Note: Can't use if-let here due to borrow checker
        let has_children = block.children().is_some();
        if has_children {
            if let Some(children) = block.children_mut() {
                if let Some(found) = find_block_recursive_mut(children, label) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn collect_labels_recursive(blocks: &[Block]) -> Vec<String> {
    let mut labels = Vec::new();
    for block in blocks {
        if let Some(label) = block.label() {
            labels.push(label.to_string());
        }
        if let Some(children) = block.children() {
            labels.extend(collect_labels_recursive(children));
        }
    }
    labels
}

fn remove_block_recursive(blocks: &mut Vec<Block>, label: &str) -> Option<Block> {
    // First, try to find and remove from this level
    for (i, block) in blocks.iter().enumerate() {
        if block.label() == Some(label) {
            return Some(blocks.remove(i));
        }
    }

    // If not found at this level, search children
    for block in blocks.iter_mut() {
        if let Some(children) = block.children_mut() {
            if let Some(removed) = remove_block_recursive(children, label) {
                return Some(removed);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::block::{InlineContent, Paragraph, Section};

    #[test]
    fn test_count_blocks() {
        let doc = Document {
            db_id: None,
            id: Some("doc1".to_string()),
            metadata: DocumentMetadata {
                title: "Test".to_string(),
                version: "1.0".to_string(),
                author: None,
                created_at: None,
                modified_at: None,
                blocks_count: None,
                tags: None,
                custom_fields: None,
            },
            docjll: vec![
                Block::Paragraph(Paragraph {
                    content: vec![],
                    label: Some("para:1".to_string()),
                    compliance_note: None,
                }),
                Block::Section(Section {
                    title: "Section 1".to_string(),
                    label: Some("sec:1".to_string()),
                    children: vec![Block::Paragraph(Paragraph {
                        content: vec![],
                        label: Some("para:2".to_string()),
                        compliance_note: None,
                    })],
                }),
            ],
        };

        assert_eq!(doc.count_blocks(), 3); // 1 paragraph + 1 section + 1 nested paragraph
    }

    #[test]
    fn test_find_block() {
        let doc = Document {
            db_id: None,
            id: Some("doc1".to_string()),
            metadata: DocumentMetadata {
                title: "Test".to_string(),
                version: "1.0".to_string(),
                author: None,
                created_at: None,
                modified_at: None,
                blocks_count: None,
                tags: None,
                custom_fields: None,
            },
            docjll: vec![
                Block::Paragraph(Paragraph {
                    content: vec![InlineContent::Text {
                        content: "Top level".to_string(),
                    }],
                    label: Some("para:1".to_string()),
                    compliance_note: None,
                }),
                Block::Section(Section {
                    title: "Section 1".to_string(),
                    label: Some("sec:1".to_string()),
                    children: vec![Block::Paragraph(Paragraph {
                        content: vec![InlineContent::Text {
                            content: "Nested".to_string(),
                        }],
                        label: Some("para:2".to_string()),
                        compliance_note: None,
                    })],
                }),
            ],
        };

        assert!(doc.find_block("para:1").is_some());
        assert!(doc.find_block("sec:1").is_some());
        assert!(doc.find_block("para:2").is_some()); // Nested
        assert!(doc.find_block("para:99").is_none());
    }

    #[test]
    fn test_collect_labels() {
        let doc = Document {
            db_id: None,
            id: Some("doc1".to_string()),
            metadata: DocumentMetadata {
                title: "Test".to_string(),
                version: "1.0".to_string(),
                author: None,
                created_at: None,
                modified_at: None,
                blocks_count: None,
                tags: None,
                custom_fields: None,
            },
            docjll: vec![
                Block::Paragraph(Paragraph {
                    content: vec![],
                    label: Some("para:1".to_string()),
                    compliance_note: None,
                }),
                Block::Section(Section {
                    title: "Section 1".to_string(),
                    label: Some("sec:1".to_string()),
                    children: vec![Block::Paragraph(Paragraph {
                        content: vec![],
                        label: Some("para:2".to_string()),
                        compliance_note: None,
                    })],
                }),
            ],
        };

        let labels = doc.collect_labels();
        assert_eq!(labels, vec!["para:1", "sec:1", "para:2"]);
    }
}
