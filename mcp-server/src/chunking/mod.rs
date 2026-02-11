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

/// Adjust byte offset to the nearest UTF-8 character boundary (backwards).
///
/// Prevents panics when slicing `&str` at positions that fall inside
/// multi-byte characters (e.g., 'í' = 2 bytes: 0xC3 0xAD).
pub(crate) fn safe_byte_offset(content: &str, mut offset: usize) -> usize {
    offset = offset.min(content.len());
    while offset > 0 && !content.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// Convert byte offset to character offset in a UTF-8 string.
///
/// Uses `safe_byte_offset` internally to handle non-boundary positions.
pub(crate) fn byte_to_char_offset(content: &str, byte_offset: usize) -> usize {
    let safe = safe_byte_offset(content, byte_offset);
    content[..safe].chars().count()
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

    #[test]
    fn test_safe_byte_offset_on_char_boundary() {
        let content = "hello";
        assert_eq!(safe_byte_offset(content, 0), 0);
        assert_eq!(safe_byte_offset(content, 3), 3);
        assert_eq!(safe_byte_offset(content, 5), 5);
    }

    #[test]
    fn test_safe_byte_offset_inside_multibyte_char() {
        // 'í' = 2 bytes (0xC3 0xAD), 'á' = 2 bytes (0xC3 0xA1)
        let content = "szív"; // s(1) z(1) í(2) v(1) = 5 bytes
        assert_eq!(content.len(), 5);
        // byte 2 = start of 'í' (valid boundary)
        assert_eq!(safe_byte_offset(content, 2), 2);
        // byte 3 = inside 'í' → snap back to 2
        assert_eq!(safe_byte_offset(content, 3), 2);
        // byte 4 = start of 'v' (valid boundary)
        assert_eq!(safe_byte_offset(content, 4), 4);
    }

    #[test]
    fn test_safe_byte_offset_beyond_length() {
        let content = "abc";
        assert_eq!(safe_byte_offset(content, 100), 3);
    }

    #[test]
    fn test_byte_to_char_offset_with_multibyte() {
        let content = "szív"; // 4 chars, 5 bytes
        assert_eq!(byte_to_char_offset(content, 0), 0); // before 's'
        assert_eq!(byte_to_char_offset(content, 2), 2); // before 'í'
        assert_eq!(byte_to_char_offset(content, 3), 2); // inside 'í' → snaps to before 'í'
        assert_eq!(byte_to_char_offset(content, 4), 3); // before 'v'
        assert_eq!(byte_to_char_offset(content, 5), 4); // end
    }

    #[test]
    fn test_text_chunking_with_hungarian_text() {
        // Regression test: overlap subtraction must not land inside multi-byte chars
        let content = "Egyszer volt, hol nem volt, még az Óperenciás-tengeren is túl, \
                        élt egy szegény öregasszony. Volt neki három fia, akik közül \
                        a legkisebbik volt a legügyesebb. Elment világgá szerencsét próbálni.";
        let result = text::split(content, 80, 20);
        assert!(result.is_ok(), "Hungarian text chunking panicked: {:?}", result.err());
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_markdown_chunking_with_hungarian_text() {
        let content = "# Népmese\n\nEgyszer volt, hol nem volt, élt egy szegény király. \
                        A királynak volt három szép lánya, akik közül a legkisebbik \
                        volt a legszebb.\n\n## Második fejezet\n\nElment a királyfi \
                        az üveghegyen túlra, ahol az árvácskák nőttek.";
        let result = markdown::split(content, 80);
        assert!(result.is_ok(), "Hungarian markdown chunking panicked: {:?}", result.err());
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());
    }
}
