// DOCJL block types and structures

use serde::{Deserialize, Serialize};

/// A DOCJL block - can be a paragraph, heading, table, list, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Block {
    Paragraph(Paragraph),
    Heading(Heading),
    Table(Table),
    List(List),
    Section(Section),
    Image(Image),
    Code(Code),
}

impl Block {
    /// Get the label of this block (if it has one)
    pub fn label(&self) -> Option<&str> {
        match self {
            Block::Paragraph(p) => p.label.as_deref(),
            Block::Heading(h) => h.label.as_deref(),
            Block::Table(t) => t.label.as_deref(),
            Block::List(l) => l.label.as_deref(),
            Block::Section(s) => s.label.as_deref(),
            Block::Image(i) => i.label.as_deref(),
            Block::Code(c) => c.label.as_deref(),
        }
    }

    /// Set the label of this block
    pub fn set_label(&mut self, label: String) {
        match self {
            Block::Paragraph(p) => p.label = Some(label),
            Block::Heading(h) => h.label = Some(label),
            Block::Table(t) => t.label = Some(label),
            Block::List(l) => l.label = Some(label),
            Block::Section(s) => s.label = Some(label),
            Block::Image(i) => i.label = Some(label),
            Block::Code(c) => c.label = Some(label),
        }
    }

    /// Get children blocks (for hierarchical blocks)
    pub fn children(&self) -> Option<&[Block]> {
        match self {
            Block::Heading(h) => h.children.as_deref(),
            Block::Section(s) => Some(&s.children),
            _ => None,
        }
    }

    /// Get mutable children blocks
    pub fn children_mut(&mut self) -> Option<&mut Vec<Block>> {
        match self {
            Block::Heading(h) => h.children.as_mut(),
            Block::Section(s) => Some(&mut s.children),
            _ => None,
        }
    }

    /// Extract all cross-references from this block
    pub fn extract_references(&self) -> Vec<String> {
        let mut refs = Vec::new();

        match self {
            Block::Paragraph(p) => {
                for content in &p.content {
                    if let InlineContent::Ref { target } = content {
                        refs.push(target.clone());
                    }
                }
            }
            Block::Heading(h) => {
                for content in &h.content {
                    if let InlineContent::Ref { target } = content {
                        refs.push(target.clone());
                    }
                }
            }
            _ => {}
        }

        refs
    }

    /// Get block type as enum
    pub fn block_type(&self) -> BlockType {
        match self {
            Block::Paragraph(_) => BlockType::Paragraph,
            Block::Heading(_) => BlockType::Heading,
            Block::Table(_) => BlockType::Table,
            Block::List(_) => BlockType::List,
            Block::Section(_) => BlockType::Section,
            Block::Image(_) => BlockType::Image,
            Block::Code(_) => BlockType::Code,
        }
    }
}

/// Block type enumeration (for queries)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockType {
    Paragraph,
    Heading,
    Table,
    List,
    Section,
    Image,
    Code,
}

/// Paragraph block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paragraph {
    pub content: Vec<InlineContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compliance_note: Option<String>,
}

/// Heading block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,  // 1-6
    pub content: Vec<InlineContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Block>>,
}

/// Table block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// List block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    pub ordered: bool,
    pub items: Vec<ListItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub content: Vec<InlineContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ListItem>>,
}

/// Section block (container)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub children: Vec<Block>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Image block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Image {
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Code block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Code {
    pub language: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Inline content (for paragraphs and headings)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InlineContent {
    Text { content: String },
    Bold { content: String },
    Italic { content: String },
    Code { content: String },
    Link { href: String, content: String },
    Ref { target: String },  // Cross-reference
}

impl InlineContent {
    /// Extract plain text from inline content
    pub fn to_text(&self) -> String {
        match self {
            InlineContent::Text { content }
            | InlineContent::Bold { content }
            | InlineContent::Italic { content }
            | InlineContent::Code { content }
            | InlineContent::Link { content, .. } => content.clone(),
            InlineContent::Ref { target } => format!("[ref:{}]", target),
        }
    }
}

/// Helper to convert inline content to plain text
pub fn inline_to_plain_text(content: &[InlineContent]) -> String {
    content.iter().map(|c| c.to_text()).collect::<Vec<_>>().join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_label() {
        let mut block = Block::Paragraph(Paragraph {
            content: vec![InlineContent::Text {
                content: "Test".to_string(),
            }],
            label: Some("para:1".to_string()),
            compliance_note: None,
        });

        assert_eq!(block.label(), Some("para:1"));

        block.set_label("para:2".to_string());
        assert_eq!(block.label(), Some("para:2"));
    }

    #[test]
    fn test_extract_references() {
        let block = Block::Paragraph(Paragraph {
            content: vec![
                InlineContent::Text {
                    content: "See ".to_string(),
                },
                InlineContent::Ref {
                    target: "sec:4".to_string(),
                },
                InlineContent::Text {
                    content: " and ".to_string(),
                },
                InlineContent::Ref {
                    target: "tab:5".to_string(),
                },
            ],
            label: None,
            compliance_note: None,
        });

        let refs = block.extract_references();
        assert_eq!(refs, vec!["sec:4", "tab:5"]);
    }

    #[test]
    fn test_inline_to_plain_text() {
        let content = vec![
            InlineContent::Bold {
                content: "Important".to_string(),
            },
            InlineContent::Text {
                content: " text".to_string(),
            },
        ];

        assert_eq!(inline_to_plain_text(&content), "Important text");
    }
}
