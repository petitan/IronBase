// DOCJL block types and structures

use serde::{Deserialize, Serialize};

// Import all new block types
pub use super::block_new_types::*;

/// A DOCJL block - can be a paragraph, heading, table, list, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph(Paragraph),
    Heading(Heading),
    Table(Table),
    #[serde(rename = "list")]
    List(List),
    #[serde(rename = "list_ordered")]
    ListOrdered(List),
    #[serde(rename = "list_unordered")]
    ListUnordered(List),
    #[serde(rename = "list_description")]
    ListDescription(ListDescription),
    Section(Section),
    Image(Image),
    #[serde(rename = "code_block")]
    CodeBlock(CodeBlock),
    Equation(Equation),
    Align(Align),
    Split(Split),
    Quote(Quote),
    #[serde(rename = "horizontal_rule")]
    HorizontalRule(HorizontalRule),
    #[serde(rename = "latex_command")]
    LatexCommand(LatexCommand),
    Pagebreak(Pagebreak),
    Vspace(Vspace),
    Bibliography(Bibliography),
    Abstract(Abstract),
    Subfigure(Subfigure),
    #[serde(rename = "appendix_start")]
    AppendixStart(AppendixStart),
    #[serde(rename = "document_header")]
    DocumentHeader(DocumentHeader),
    Infobox(Infobox),
    Resultbox(Resultbox),
    #[serde(rename = "signature_table")]
    SignatureTable(SignatureTable),
    #[serde(rename = "equation_block")]
    EquationBlock(EquationBlock),
    Figure(Figure),
}

impl Block {
    /// Get the label of this block (if it has one)
    pub fn label(&self) -> Option<&str> {
        match self {
            Block::Paragraph(p) => p.label.as_deref(),
            Block::Heading(h) => h.label.as_deref(),
            Block::Table(t) => t.label.as_deref(),
            Block::List(l) | Block::ListOrdered(l) | Block::ListUnordered(l) => l.label.as_deref(),
            Block::ListDescription(l) => l.label.as_deref(),
            Block::Section(s) => s.label.as_deref(),
            Block::Image(i) => i.label.as_deref(),
            Block::CodeBlock(c) => c.label.as_deref(),
            Block::Equation(e) => e.label.as_deref(),
            Block::Align(_) => None,
            Block::Split(_) => None,
            Block::Quote(q) => q.label.as_deref(),
            Block::HorizontalRule(_) => None,
            Block::LatexCommand(_) => None,
            Block::Pagebreak(_) => None,
            Block::Vspace(_) => None,
            Block::Bibliography(b) => b.label.as_deref(),
            Block::Abstract(a) => a.label.as_deref(),
            Block::Subfigure(s) => s.label.as_deref(),
            Block::AppendixStart(_) => None,
            Block::DocumentHeader(_) => None,
            Block::Infobox(i) => i.label.as_deref(),
            Block::Resultbox(r) => r.label.as_deref(),
            Block::SignatureTable(s) => s.label.as_deref(),
            Block::EquationBlock(e) => e.label.as_deref(),
            Block::Figure(f) => f.label.as_deref(),
        }
    }

    /// Set the label of this block
    pub fn set_label(&mut self, label: String) {
        match self {
            Block::Paragraph(p) => p.label = Some(label),
            Block::Heading(h) => h.label = Some(label),
            Block::Table(t) => t.label = Some(label),
            Block::List(l) | Block::ListOrdered(l) | Block::ListUnordered(l) => l.label = Some(label),
            Block::ListDescription(l) => l.label = Some(label),
            Block::Section(s) => s.label = Some(label),
            Block::Image(i) => i.label = Some(label),
            Block::CodeBlock(c) => c.label = Some(label),
            Block::Equation(e) => e.label = Some(label),
            Block::Align(_) => {},
            Block::Split(_) => {},
            Block::Quote(q) => q.label = Some(label),
            Block::HorizontalRule(_) => {},
            Block::LatexCommand(_) => {},
            Block::Pagebreak(_) => {},
            Block::Vspace(_) => {},
            Block::Bibliography(b) => b.label = Some(label),
            Block::Abstract(a) => a.label = Some(label),
            Block::Subfigure(s) => s.label = Some(label),
            Block::AppendixStart(_) => {},
            Block::DocumentHeader(_) => {},
            Block::Infobox(i) => i.label = Some(label),
            Block::Resultbox(r) => r.label = Some(label),
            Block::SignatureTable(s) => s.label = Some(label),
            Block::EquationBlock(e) => e.label = Some(label),
            Block::Figure(f) => f.label = Some(label),
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
            Block::List(_) | Block::ListOrdered(_) | Block::ListUnordered(_) | Block::ListDescription(_) => BlockType::List,
            Block::Section(_) => BlockType::Section,
            Block::Image(_) => BlockType::Image,
            Block::CodeBlock(_) => BlockType::Code,
            Block::Equation(_) | Block::EquationBlock(_) => BlockType::Equation,
            Block::Align(_) | Block::Split(_) => BlockType::Equation,
            Block::Quote(_) => BlockType::Quote,
            Block::HorizontalRule(_) => BlockType::HorizontalRule,
            Block::LatexCommand(_) => BlockType::LatexCommand,
            Block::Pagebreak(_) => BlockType::Pagebreak,
            Block::Vspace(_) => BlockType::Vspace,
            Block::Bibliography(_) => BlockType::Bibliography,
            Block::Abstract(_) => BlockType::Abstract,
            Block::Subfigure(_) | Block::Figure(_) => BlockType::Figure,
            Block::AppendixStart(_) => BlockType::AppendixStart,
            Block::DocumentHeader(_) => BlockType::DocumentHeader,
            Block::Infobox(_) => BlockType::Infobox,
            Block::Resultbox(_) => BlockType::Resultbox,
            Block::SignatureTable(_) => BlockType::SignatureTable,
        }
    }
}

/// Block type enumeration (for queries)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Paragraph,
    Heading,
    Table,
    List,
    Section,
    Image,
    Code,
    Equation,
    Quote,
    HorizontalRule,
    LatexCommand,
    Pagebreak,
    Vspace,
    Bibliography,
    Abstract,
    Figure,
    AppendixStart,
    DocumentHeader,
    Infobox,
    Resultbox,
    SignatureTable,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,  // 1-6, optional for compatibility
    pub content: Vec<InlineContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Block>>,
}

/// Table block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    #[serde(default)]
    pub headers: Vec<String>,
    #[serde(default)]
    pub rows: Vec<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// List block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    #[serde(default)]
    pub ordered: bool,
    #[serde(default)]
    pub items: Vec<String>,  // Simplified to just strings for now, default to empty
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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
