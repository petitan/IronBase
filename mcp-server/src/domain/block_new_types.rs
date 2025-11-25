// New block types for DOCJL schema v3.1.0+
use serde::{Deserialize, Serialize};

/// List with description items (definition list)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListDescription {
    #[serde(default)]
    pub items: Vec<DescriptionItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescriptionItem {
    pub term: String,
    pub description: String,
}

/// Code block with syntax highlighting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub line_numbers: bool,
}

/// Mathematical equation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Equation {
    pub content: String,
    #[serde(default)]
    pub numbered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Align environment for multiple equations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Align {
    pub equations: Vec<String>,
    #[serde(default)]
    pub numbered: bool,
}

/// Split environment for long equations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Split {
    pub parts: Vec<String>,
    #[serde(default)]
    pub numbered: bool,
}

/// Quote block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quote {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Horizontal rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HorizontalRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// Raw LaTeX command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatexCommand {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
}

/// Page break
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagebreak {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

/// Vertical space
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vspace {
    pub size: String,
}

/// Bibliography
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bibliography {
    pub style: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Abstract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Abstract {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Subfigure (for multi-part figures)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subfigure {
    pub figures: Vec<SubfigureItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubfigureItem {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
}

/// Appendix start marker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendixStart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Document header (title, author, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentHeader {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affiliation: Option<String>,
}

/// Info box (for important information)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Infobox {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Result box (for displaying results)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resultbox {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Signature table (for approvals/signatures)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureTable {
    pub signatures: Vec<SignatureEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureEntry {
    pub role: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
}

/// Equation block (similar to equation but with more options)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquationBlock {
    pub equations: Vec<String>,
    #[serde(default)]
    pub numbered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Figure with multiple images
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Figure {
    pub images: Vec<FigureImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FigureImage {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
}