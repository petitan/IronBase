//! Text chunking module for embedding long documents
//!
//! Supports markdown-aware chunking that preserves heading structure,
//! and plain text chunking. Both modes support configurable overlap.

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
    /// Plain text chunking
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
    /// Overlap between chunks in characters
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

/// Convert character offset to byte offset in a UTF-8 string.
///
/// Returns the byte position of the `char_offset`-th character.
/// If `char_offset` exceeds the number of characters, returns `content.len()`.
pub(crate) fn char_to_byte_offset(content: &str, char_offset: usize) -> usize {
    content
        .char_indices()
        .nth(char_offset)
        .map(|(i, _)| i)
        .unwrap_or(content.len())
}

/// Calculate byte offset for the overlap start position.
///
/// Goes back `overlap_chars` characters from `chunk_start_byte` in the
/// original content. Always returns a valid UTF-8 boundary.
pub(crate) fn overlap_start_byte(
    content: &str,
    chunk_start_byte: usize,
    overlap_chars: usize,
) -> usize {
    if overlap_chars == 0 || chunk_start_byte == 0 {
        return chunk_start_byte;
    }
    let current_char = byte_to_char_offset(content, chunk_start_byte);
    let target_char = current_char.saturating_sub(overlap_chars);
    char_to_byte_offset(content, target_char)
}

/// Convert markdown table rows to plain comma-separated text.
///
/// - Separator rows (`|---|---|`) are removed entirely
/// - Data/header rows (`| a | b |`) become `a, b`
/// - Non-table lines pass through unchanged
pub fn strip_markdown_tables(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') {
            // Separator row: only pipes, dashes, colons, spaces → skip
            let is_separator = trimmed
                .chars()
                .all(|c| c == '|' || c == '-' || c == ':' || c == ' ');
            if is_separator {
                continue;
            }
            // Data/header row: split by pipe, trim cells, join with ", "
            let cells: Vec<&str> = trimmed
                .split('|')
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
                .collect();
            out.push_str(&cells.join(", "));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    // Remove trailing newline if original didn't have one
    if !text.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Split content into chunks
///
/// Uses the specified mode, or auto-detects if mode is Auto.
/// Markdown table rows are converted to plain text for better tokenization.
pub fn chunk_content(content: &str, options: &ChunkOptions) -> Result<Vec<Chunk>, ChunkError> {
    if content.is_empty() {
        return Ok(vec![]);
    }

    let mode = match options.mode {
        ChunkMode::Auto => ChunkMode::detect(content),
        other => other,
    };

    let chunks = match mode {
        ChunkMode::Markdown | ChunkMode::Auto => {
            markdown::split(content, options.chunk_size, options.overlap)?
        }
        ChunkMode::Text => text::split(content, options.chunk_size, options.overlap)?,
    };

    Ok(chunks)
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
        assert!(
            result.is_ok(),
            "Hungarian text chunking panicked: {:?}",
            result.err()
        );
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_markdown_chunking_with_hungarian_text() {
        let content = "# Népmese\n\nEgyszer volt, hol nem volt, élt egy szegény király. \
                        A királynak volt három szép lánya, akik közül a legkisebbik \
                        volt a legszebb.\n\n## Második fejezet\n\nElment a királyfi \
                        az üveghegyen túlra, ahol az árvácskák nőttek.";
        let result = markdown::split(content, 80, 20);
        assert!(
            result.is_ok(),
            "Hungarian markdown chunking panicked: {:?}",
            result.err()
        );
        let chunks = result.unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_char_to_byte_offset_ascii() {
        let content = "hello";
        assert_eq!(char_to_byte_offset(content, 0), 0);
        assert_eq!(char_to_byte_offset(content, 3), 3);
        assert_eq!(char_to_byte_offset(content, 5), 5);
        assert_eq!(char_to_byte_offset(content, 100), 5); // beyond end
    }

    #[test]
    fn test_char_to_byte_offset_with_multibyte() {
        let content = "szív"; // s(1) z(1) í(2) v(1) = 5 bytes, 4 chars
        assert_eq!(char_to_byte_offset(content, 0), 0); // before 's'
        assert_eq!(char_to_byte_offset(content, 2), 2); // before 'í'
        assert_eq!(char_to_byte_offset(content, 3), 4); // before 'v'
        assert_eq!(char_to_byte_offset(content, 4), 5); // end
    }

    #[test]
    fn test_overlap_start_byte_basic() {
        let content = "abcdefghij"; // 10 ASCII chars
                                    // Go back 3 chars from byte 7 ('h') → byte 4 ('e')
        assert_eq!(overlap_start_byte(content, 7, 3), 4);
        // Go back 0 chars → same position
        assert_eq!(overlap_start_byte(content, 7, 0), 7);
        // Go back from position 0 → stays at 0
        assert_eq!(overlap_start_byte(content, 0, 5), 0);
        // Go back more chars than available → 0
        assert_eq!(overlap_start_byte(content, 3, 100), 0);
    }

    #[test]
    fn test_overlap_start_byte_multibyte() {
        // "Helló világ" - H(1) e(1) l(1) l(1) ó(2) ' '(1) v(1) i(1) l(1) á(2) g(1)
        // = 12 bytes, 11 chars
        let content = "Helló világ";
        assert_eq!(content.len(), 13); // actual byte len
                                       // Go back 3 chars from byte 7 (which is 'v', char index 6) → char 3 = 'l' = byte 3
        assert_eq!(overlap_start_byte(content, 7, 3), 3);
    }

    #[test]
    fn test_text_overlap_produces_overlapping_content() {
        let content = "Alpha bravo charlie. Delta echo foxtrot. \
                        Golf hotel india. Juliet kilo lima.";
        let chunks = text::split(content, 40, 10).unwrap();
        if chunks.len() >= 2 {
            // Verify chunk 1 starts before chunk 0 ends (overlap)
            assert!(
                chunks[1].start_char < chunks[0].end_char,
                "Expected overlap: chunk[1].start_char={} should be < chunk[0].end_char={}",
                chunks[1].start_char,
                chunks[0].end_char
            );
            // Verify the overlapping text is shared
            let overlap_len = chunks[0].end_char - chunks[1].start_char;
            assert!(
                overlap_len > 0,
                "Overlap should be positive, got {}",
                overlap_len
            );
        }
    }

    #[test]
    fn test_text_no_overlap_produces_no_overlapping_content() {
        let content = "Alpha bravo charlie. Delta echo foxtrot. \
                        Golf hotel india. Juliet kilo lima.";
        let chunks = text::split(content, 40, 0).unwrap();
        if chunks.len() >= 2 {
            // With overlap=0, chunk 1 should not start before chunk 0 ends
            assert!(
                chunks[1].start_char >= chunks[0].end_char,
                "No overlap expected: chunk[1].start_char={} should be >= chunk[0].end_char={}",
                chunks[1].start_char,
                chunks[0].end_char
            );
        }
    }

    #[test]
    fn test_markdown_overlap_produces_overlapping_content() {
        let content = "# Title\n\nFirst paragraph with some content here. \
                        More text to fill the chunk.\n\n## Section\n\n\
                        Second paragraph with different content. Even more text here.";
        let chunks = markdown::split(content, 60, 15).unwrap();
        if chunks.len() >= 2 {
            assert!(
                chunks[1].start_char < chunks[0].end_char,
                "Expected markdown overlap: chunk[1].start_char={} < chunk[0].end_char={}",
                chunks[1].start_char,
                chunks[0].end_char
            );
        }
    }

    #[test]
    fn test_strip_markdown_table_basic() {
        let input = "| Me. | Megnevezés | Nettó Ft/db |\n|---|---|---|\n| 1 | PEF-35 fékerőmérő | 1 750 000 Ft |";
        let result = strip_markdown_tables(input);
        assert_eq!(
            result,
            "Me., Megnevezés, Nettó Ft/db\n1, PEF-35 fékerőmérő, 1 750 000 Ft"
        );
    }

    #[test]
    fn test_strip_markdown_table_separator_removed() {
        let input = "|---|---|---|\n| a | b |";
        let result = strip_markdown_tables(input);
        assert_eq!(result, "a, b");
    }

    #[test]
    fn test_strip_markdown_table_mixed_content() {
        let input = "Some text before\n| A | B |\n|---|---|\n| 1 | 2 |\nSome text after";
        let result = strip_markdown_tables(input);
        assert_eq!(result, "Some text before\nA, B\n1, 2\nSome text after");
    }

    #[test]
    fn test_strip_markdown_table_no_table() {
        let input = "Just plain text\nwith multiple lines";
        let result = strip_markdown_tables(input);
        assert_eq!(result, input);
    }
}
