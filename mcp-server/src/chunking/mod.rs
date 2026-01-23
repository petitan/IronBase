//! Text chunking module for embedding long documents
//!
//! Supports markdown-aware chunking that preserves heading structure,
//! and plain text chunking with configurable overlap.

pub mod markdown;
pub mod text;

use serde::{Deserialize, Serialize};

/// A chunk of text with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Chunk index (0-based)
    pub index: usize,
    /// Total number of chunks
    pub total: usize,
    /// The actual text content
    pub text: String,
    /// Character offset in original document (start)
    pub start_char: usize,
    /// Character offset in original document (end)
    pub end_char: usize,
    /// Optional heading at the start of this chunk
    pub heading: Option<String>,
    /// Heading level (1-6) if heading is present
    pub heading_level: Option<u8>,
    /// Section path (e.g., ["Chapter 1", "Section 1.1"])
    pub section_path: Option<Vec<String>>,
}

impl Chunk {
    /// Create a new chunk
    pub fn new(
        index: usize,
        total: usize,
        text: String,
        start_char: usize,
        end_char: usize,
    ) -> Self {
        Self {
            index,
            total,
            text,
            start_char,
            end_char,
            heading: None,
            heading_level: None,
            section_path: None,
        }
    }

    /// Add heading metadata
    pub fn with_heading(mut self, heading: String, level: u8) -> Self {
        self.heading = Some(heading);
        self.heading_level = Some(level);
        self
    }

    /// Add section path
    pub fn with_section_path(mut self, path: Vec<String>) -> Self {
        self.section_path = Some(path);
        self
    }
}

/// Chunking mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChunkMode {
    /// Auto-detect based on content (markdown if starts with # or has ## headings)
    #[default]
    Auto,
    /// Markdown-aware chunking (respects headings)
    Markdown,
    /// Plain text chunking with overlap
    Text,
}

impl ChunkMode {
    /// Parse mode from string
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Self::Markdown,
            "text" | "plain" => Self::Text,
            _ => Self::Auto,
        }
    }

    /// Detect mode from content
    pub fn detect(content: &str) -> Self {
        // Check for markdown indicators
        let lines: Vec<&str> = content.lines().take(20).collect();

        // Look for markdown headings
        let has_headings = lines.iter().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with('#') && trimmed.len() > 1
        });

        if has_headings {
            Self::Markdown
        } else {
            Self::Text
        }
    }
}

/// Chunking options
#[derive(Debug, Clone)]
pub struct ChunkOptions {
    /// Maximum chunk size in characters
    pub chunk_size: usize,
    /// Overlap between chunks in characters (for text mode)
    pub overlap: usize,
    /// Chunking mode
    pub mode: ChunkMode,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            chunk_size: 1000,
            overlap: 100,
            mode: ChunkMode::Auto,
        }
    }
}

impl ChunkOptions {
    /// Create with custom chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Create with custom overlap
    pub fn with_overlap(mut self, overlap: usize) -> Self {
        self.overlap = overlap;
        self
    }

    /// Create with specific mode
    pub fn with_mode(mut self, mode: ChunkMode) -> Self {
        self.mode = mode;
        self
    }
}

/// Split content into chunks
///
/// Uses the specified mode, or auto-detects if mode is Auto.
pub fn chunk_content(content: &str, options: &ChunkOptions) -> Result<Vec<Chunk>, ChunkError> {
    if content.is_empty() {
        return Ok(vec![]);
    }

    let mode = match options.mode {
        ChunkMode::Auto => ChunkMode::detect(content),
        other => other,
    };

    match mode {
        ChunkMode::Markdown | ChunkMode::Auto => markdown::split(content, options.chunk_size),
        ChunkMode::Text => text::split(content, options.chunk_size, options.overlap),
    }
}

/// Errors that can occur during chunking
#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("Content is too large: {size} characters (max: {max})")]
    ContentTooLarge { size: usize, max: usize },

    #[error("Failed to allocate memory for {count} chunks")]
    OutOfMemory { count: usize },

    #[error("Invalid chunk options: {0}")]
    InvalidOptions(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_detection_markdown() {
        let content = "# Title\n\nSome content\n\n## Section";
        assert_eq!(ChunkMode::detect(content), ChunkMode::Markdown);
    }

    #[test]
    fn test_mode_detection_text() {
        let content = "Just some plain text\nwithout any markdown";
        assert_eq!(ChunkMode::detect(content), ChunkMode::Text);
    }

    #[test]
    fn test_chunk_empty_content() {
        let options = ChunkOptions::default();
        let chunks = chunk_content("", &options).unwrap();
        assert!(chunks.is_empty());
    }
}
