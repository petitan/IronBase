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
    /// For a chunk that continues a table without its own header, the table's
    /// "<header>\n<separator>" block that was prepended to `text` (#63). The
    /// adjacent-chunk merge uses this to strip the duplicate exactly (no
    /// re-derivation / string-guessing). `None` for header-bearing or non-table chunks.
    pub table_header: Option<String>,
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
            table_header: None,
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

    /// Record the propagated table header (the block prepended to `text`).
    pub fn with_table_header(mut self, header: String) -> Self {
        self.table_header = Some(header);
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
/// original content, then snaps forward to the nearest word boundary
/// (whitespace) so overlapping text doesn't start mid-word.
/// Always returns a valid UTF-8 boundary.
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
    let target_byte = char_to_byte_offset(content, target_char);

    // Snap forward to nearest word boundary (whitespace)
    // so the overlap doesn't start in the middle of a word
    if target_byte < chunk_start_byte {
        let slice = &content[target_byte..chunk_start_byte];
        // If we're already at a word boundary, keep it
        if slice.starts_with(|c: char| c.is_whitespace()) {
            return target_byte;
        }
        // Find the next whitespace in the slice and snap to it
        if let Some(ws_offset) = slice.find(|c: char| c.is_whitespace()) {
            // Skip the whitespace itself to start at the word
            let after_ws = target_byte + ws_offset;
            let snapped = content[after_ws..]
                .find(|c: char| !c.is_whitespace())
                .map(|off| after_ws + off)
                .unwrap_or(after_ws);
            if snapped < chunk_start_byte {
                return snapped;
            }
        }
    }
    target_byte
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
            // Separator row (e.g. `|---|:--:|`) → skip. Shared predicate so this
            // and the table-header detection classify rows identically (#65 review).
            if is_table_separator(line) {
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

/// True if a line is a markdown table row (starts with `|`).
pub(crate) fn is_table_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

/// True if a line is a markdown table separator row (e.g. `|---|:--:|`).
pub(crate) fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Find a table header block (header row + separator row) anywhere in `text`,
/// returning `"<header>\n<separator>"` using the original line text. Returns the
/// last such block found (the one a following chunk most likely continues).
///
/// Used for table header propagation (#63): a chunk that continues a table but
/// holds no separator of its own gets this block prepended, so the values stay
/// interpretable; the adjacent-chunk merge strips the duplicate.
pub(crate) fn find_table_header(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = None;
    for i in 1..lines.len() {
        if is_table_separator(lines[i])
            && is_table_row(lines[i - 1])
            && !is_table_separator(lines[i - 1])
        {
            found = Some(format!("{}\n{}", lines[i - 1], lines[i]));
        }
    }
    found
}

/// Build the text used to embed a chunk.
///
/// Combines the chunk body with its section breadcrumb so the embedding vector
/// carries hierarchical context (e.g. `"Brakes > PEF-35 > Specs\n\n<body>"`).
/// Markdown tables in the body are flattened to plain text for cleaner
/// tokenization.
///
/// This is used ONLY for embedding — the original chunk text is stored and
/// returned unchanged. Generic auto-embedding (auto_embed.rs / crud.rs) embeds
/// the source field verbatim and intentionally does not call this.
pub fn build_embed_text(body: &str, section_path: Option<&[String]>) -> String {
    let clean = strip_markdown_tables(body);
    match section_path {
        Some(path) if !path.is_empty() => format!("{}\n\n{}", path.join(" > "), clean),
        _ => clean,
    }
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
        let content = "alpha bravo charlie delta";
        // Go back 6 chars from byte 12 ('c') → lands at "bravo " → snaps to 'b'
        let result = overlap_start_byte(content, 12, 6);
        assert!(
            content[result..].starts_with(|c: char| !c.is_whitespace()),
            "Overlap should start at a word, got: '{}'",
            &content[result..result + 5.min(content.len() - result)]
        );
        // Go back 0 chars → same position
        assert_eq!(overlap_start_byte(content, 12, 0), 12);
        // Go back from position 0 → stays at 0
        assert_eq!(overlap_start_byte(content, 0, 5), 0);
    }

    #[test]
    fn test_overlap_start_byte_snaps_to_word() {
        let content = "fékerőmérő kalibrálási jegyzőkönyv elkészítése";
        // Go back from "jegyzőkönyv" start — should snap to a word boundary
        let jegyzo_start = content.find("jegyző").unwrap();
        let result = overlap_start_byte(content, jegyzo_start, 5);
        let overlap_text = &content[result..jegyzo_start];
        // Should not start mid-word
        assert!(
            result == 0
                || content[..result].ends_with(|c: char| c.is_whitespace())
                || content[result..].starts_with(|c: char| !c.is_whitespace()),
            "Overlap should start at word boundary, got offset {} = '{}'",
            result,
            overlap_text
        );
    }

    #[test]
    fn test_overlap_start_byte_multibyte() {
        let content = "Helló világ";
        // Go back 3 chars from 'v' (byte 7) → snaps to word boundary
        let result = overlap_start_byte(content, 7, 3);
        assert!(result <= 7);
        // Should land at a valid position
        assert!(content.is_char_boundary(result));
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

    #[test]
    fn test_build_embed_text_with_section_path() {
        let path = vec!["Fékpadok".to_string(), "PEF-35".to_string()];
        let result = build_embed_text("A műszaki adatok.", Some(&path));
        assert_eq!(result, "Fékpadok > PEF-35\n\nA műszaki adatok.");
    }

    #[test]
    fn test_build_embed_text_without_section_path() {
        assert_eq!(build_embed_text("Plain body.", None), "Plain body.");
        // Empty path behaves like None (no breadcrumb prefix)
        let empty: Vec<String> = vec![];
        assert_eq!(build_embed_text("Plain body.", Some(&empty)), "Plain body.");
    }

    #[test]
    fn test_build_embed_text_flattens_table() {
        let body = "| A | B |\n|---|---|\n| 1 | 2 |";
        assert_eq!(build_embed_text(body, None), "A, B\n1, 2");
    }

    #[test]
    fn test_build_embed_text_table_with_breadcrumb() {
        let path = vec!["Árlista".to_string()];
        let body = "| Megnevezés | Ár |\n|---|---|\n| PEF-35 | 1750000 |";
        assert_eq!(
            build_embed_text(body, Some(&path)),
            "Árlista\n\nMegnevezés, Ár\nPEF-35, 1750000"
        );
    }
}
