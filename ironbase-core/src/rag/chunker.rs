//! Smart Markdown chunker for RAG document processing
//!
//! Features:
//! - Preserves tables as single chunks (critical for ISO documents)
//! - Preserves code blocks as single chunks
//! - Tracks section hierarchy from headings
//! - Token-based chunking with overlap
//! - Splits on sentence boundaries when possible
//!
//! # Example
//!
//! ```rust,ignore
//! let chunker = Chunker::new(ChunkConfig::default());
//! let chunks = chunker.chunk_markdown(markdown_text);
//!
//! for chunk in chunks {
//!     println!("Section: {:?}", chunk.section_path);
//!     println!("Type: {:?}", chunk.block_type);
//!     println!("Tokens: {}", chunk.token_count);
//!     println!("---");
//! }
//! ```

use super::types::{BlockType, Chunk, ChunkConfig};
use std::collections::VecDeque;

/// Markdown block with metadata
#[derive(Debug, Clone)]
struct MarkdownBlock {
    /// Block type
    block_type: BlockType,
    /// Raw text content
    text: String,
    /// Character range in original document
    char_range: (usize, usize),
    /// Heading level (1-6) if this is a heading
    heading_level: Option<u8>,
}

/// Smart Markdown chunker that preserves document structure
pub struct Chunker {
    config: ChunkConfig,
}

impl Chunker {
    /// Create a new chunker with the given configuration
    pub fn new(config: ChunkConfig) -> Self {
        Self { config }
    }

    /// Create chunker with default configuration
    pub fn with_defaults() -> Self {
        Self::new(ChunkConfig::default())
    }

    /// Chunk a markdown document into semantic chunks
    pub fn chunk_markdown(&self, text: &str) -> Vec<Chunk> {
        // Step 1: Parse markdown into blocks
        let blocks = self.parse_blocks(text);

        // Step 2: Build section hierarchy and create chunks
        self.process_blocks(blocks)
    }

    /// Parse markdown text into blocks
    fn parse_blocks(&self, text: &str) -> Vec<MarkdownBlock> {
        let mut blocks = Vec::new();
        let mut current_pos = 0;
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let line_start = text[current_pos..]
                .find(line)
                .map(|p| current_pos + p)
                .unwrap_or(current_pos);

            // Check for code block (fenced)
            if line.starts_with("```") || line.starts_with("~~~") {
                let fence = if line.starts_with("```") {
                    "```"
                } else {
                    "~~~"
                };
                let block_start = line_start;
                let mut block_text = String::new();
                block_text.push_str(line);
                block_text.push('\n');
                i += 1;

                // Find closing fence
                while i < lines.len() {
                    let inner_line = lines[i];
                    block_text.push_str(inner_line);
                    block_text.push('\n');
                    i += 1;
                    if inner_line.trim().starts_with(fence) {
                        break;
                    }
                }

                let block_end = block_start + block_text.len();
                blocks.push(MarkdownBlock {
                    block_type: BlockType::Code,
                    text: block_text.trim_end().to_string(),
                    char_range: (block_start, block_end),
                    heading_level: None,
                });
                current_pos = block_end;
                continue;
            }

            // Check for table (starts with |)
            if line.trim().starts_with('|') && line.contains('|') {
                let block_start = line_start;
                let mut table_text = String::new();

                while i < lines.len() && lines[i].trim().starts_with('|') {
                    table_text.push_str(lines[i]);
                    table_text.push('\n');
                    i += 1;
                }

                let block_end = block_start + table_text.len();
                blocks.push(MarkdownBlock {
                    block_type: BlockType::Table,
                    text: table_text.trim_end().to_string(),
                    char_range: (block_start, block_end),
                    heading_level: None,
                });
                current_pos = block_end;
                continue;
            }

            // Check for heading
            if line.starts_with('#') {
                let level = line.chars().take_while(|&c| c == '#').count() as u8;
                if level <= 6 {
                    let heading_text = line.trim_start_matches('#').trim().to_string();
                    let block_end = line_start + line.len();

                    blocks.push(MarkdownBlock {
                        block_type: BlockType::Heading,
                        text: heading_text,
                        char_range: (line_start, block_end),
                        heading_level: Some(level),
                    });
                    current_pos = block_end;
                    i += 1;
                    continue;
                }
            }

            // Check for blockquote
            if line.starts_with('>') {
                let block_start = line_start;
                let mut quote_text = String::new();

                while i < lines.len() && (lines[i].starts_with('>') || lines[i].trim().is_empty()) {
                    if lines[i].trim().is_empty()
                        && i + 1 < lines.len()
                        && !lines[i + 1].starts_with('>')
                    {
                        break;
                    }
                    let clean_line = lines[i].trim_start_matches('>').trim_start();
                    quote_text.push_str(clean_line);
                    quote_text.push('\n');
                    i += 1;
                }

                let block_end = block_start + quote_text.len();
                blocks.push(MarkdownBlock {
                    block_type: BlockType::Quote,
                    text: quote_text.trim_end().to_string(),
                    char_range: (block_start, block_end),
                    heading_level: None,
                });
                current_pos = block_end;
                continue;
            }

            // Check for list item
            if is_list_item(line) {
                let block_start = line_start;
                let mut list_text = String::new();

                while i < lines.len() {
                    let current = lines[i];
                    // Continue if it's a list item or indented continuation
                    if is_list_item(current) || (current.starts_with("  ") && !list_text.is_empty())
                    {
                        list_text.push_str(current);
                        list_text.push('\n');
                        i += 1;
                    } else if current.trim().is_empty()
                        && i + 1 < lines.len()
                        && is_list_item(lines[i + 1])
                    {
                        // Empty line between list items
                        list_text.push('\n');
                        i += 1;
                    } else {
                        break;
                    }
                }

                let block_end = block_start + list_text.len();
                blocks.push(MarkdownBlock {
                    block_type: BlockType::List,
                    text: list_text.trim_end().to_string(),
                    char_range: (block_start, block_end),
                    heading_level: None,
                });
                current_pos = block_end;
                continue;
            }

            // Skip empty lines
            if line.trim().is_empty() {
                i += 1;
                current_pos = line_start + line.len() + 1;
                continue;
            }

            // Regular paragraph
            let block_start = line_start;
            let mut para_text = String::new();

            while i < lines.len() {
                let current = lines[i];
                // End paragraph on empty line, heading, list, table, code, or quote
                if current.trim().is_empty()
                    || current.starts_with('#')
                    || current.starts_with("```")
                    || current.starts_with("~~~")
                    || current.trim().starts_with('|')
                    || current.starts_with('>')
                    || is_list_item(current)
                {
                    break;
                }
                para_text.push_str(current);
                para_text.push(' ');
                i += 1;
            }

            let block_end = block_start + para_text.len();
            blocks.push(MarkdownBlock {
                block_type: BlockType::Paragraph,
                text: para_text.trim_end().to_string(),
                char_range: (block_start, block_end),
                heading_level: None,
            });
            current_pos = block_end;
        }

        blocks
    }

    /// Process blocks into chunks with section tracking
    fn process_blocks(&self, blocks: Vec<MarkdownBlock>) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut section_path: Vec<String> = Vec::new();
        let mut section_levels: Vec<u8> = Vec::new();
        let mut pending_text = String::new();
        let mut pending_start = 0;
        let mut pending_type = BlockType::Paragraph;

        for block in blocks {
            // Update section hierarchy for headings
            if block.block_type == BlockType::Heading {
                if let Some(level) = block.heading_level {
                    // Flush pending content before heading
                    if !pending_text.is_empty() {
                        self.add_chunks(
                            &mut chunks,
                            &pending_text,
                            pending_start,
                            pending_type,
                            &section_path,
                        );
                        pending_text.clear();
                    }

                    // Update section path
                    while !section_levels.is_empty() && *section_levels.last().unwrap() >= level {
                        section_levels.pop();
                        section_path.pop();
                    }
                    section_levels.push(level);
                    section_path.push(block.text.clone());

                    // Don't add heading as separate chunk if split_on_heading is false
                    if !self.config.split_on_heading {
                        continue;
                    }
                }
            }

            // Handle preserved blocks (tables and code)
            let preserve = (block.block_type == BlockType::Table && self.config.preserve_tables)
                || (block.block_type == BlockType::Code && self.config.preserve_code);

            if preserve {
                // Flush pending content
                if !pending_text.is_empty() {
                    self.add_chunks(
                        &mut chunks,
                        &pending_text,
                        pending_start,
                        pending_type,
                        &section_path,
                    );
                    pending_text.clear();
                }

                // Add preserved block as single chunk
                chunks.push(Chunk {
                    text: block.text,
                    char_range: block.char_range,
                    token_count: estimate_tokens(&pending_text), // Will be recalculated
                    block_type: block.block_type,
                    section_path: section_path.clone(),
                    parent_heading: section_path.last().cloned(),
                });
            } else {
                // Accumulate text for chunking
                if pending_text.is_empty() {
                    pending_start = block.char_range.0;
                    pending_type = block.block_type;
                } else {
                    pending_text.push_str("\n\n");
                }
                pending_text.push_str(&block.text);
            }
        }

        // Flush remaining content
        if !pending_text.is_empty() {
            self.add_chunks(
                &mut chunks,
                &pending_text,
                pending_start,
                pending_type,
                &section_path,
            );
        }

        // Recalculate token counts
        for chunk in &mut chunks {
            chunk.token_count = estimate_tokens(&chunk.text);
        }

        chunks
    }

    /// Add text as one or more chunks (splitting if needed)
    fn add_chunks(
        &self,
        chunks: &mut Vec<Chunk>,
        text: &str,
        start_pos: usize,
        block_type: BlockType,
        section_path: &[String],
    ) {
        let tokens = estimate_tokens(text);

        if tokens <= self.config.max_tokens {
            // Single chunk
            chunks.push(Chunk {
                text: text.to_string(),
                char_range: (start_pos, start_pos + text.len()),
                token_count: tokens,
                block_type,
                section_path: section_path.to_vec(),
                parent_heading: section_path.last().cloned(),
            });
        } else {
            // Split into multiple chunks with overlap
            let split_chunks = self.split_text(text, start_pos, block_type, section_path);
            chunks.extend(split_chunks);
        }
    }

    /// Split text into chunks with overlap
    fn split_text(
        &self,
        text: &str,
        base_pos: usize,
        block_type: BlockType,
        section_path: &[String],
    ) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let sentences = split_sentences(text);
        let mut current_chunk = String::new();
        let mut current_start = 0;
        let mut overlap_buffer: VecDeque<String> = VecDeque::new();

        for sentence in sentences {
            let _sentence_tokens = estimate_tokens(&sentence);

            // Check if adding this sentence would exceed max tokens
            let combined = if current_chunk.is_empty() {
                sentence.clone()
            } else {
                format!("{} {}", current_chunk, sentence)
            };
            let combined_tokens = estimate_tokens(&combined);

            if combined_tokens > self.config.max_tokens && !current_chunk.is_empty() {
                // Save current chunk
                let token_count = estimate_tokens(&current_chunk);
                if token_count >= self.config.min_chunk_size {
                    chunks.push(Chunk {
                        text: current_chunk.clone(),
                        char_range: (
                            base_pos + current_start,
                            base_pos + current_start + current_chunk.len(),
                        ),
                        token_count,
                        block_type,
                        section_path: section_path.to_vec(),
                        parent_heading: section_path.last().cloned(),
                    });
                }

                // Start new chunk with overlap
                current_start = current_start + current_chunk.len() - self.overlap_chars();
                current_chunk = overlap_buffer.iter().cloned().collect::<Vec<_>>().join(" ");
                overlap_buffer.clear();
            }

            // Add sentence to current chunk
            if current_chunk.is_empty() {
                current_chunk = sentence.clone();
            } else {
                current_chunk = format!("{} {}", current_chunk, sentence);
            }

            // Maintain overlap buffer
            overlap_buffer.push_back(sentence);
            while estimate_tokens(&overlap_buffer.iter().cloned().collect::<Vec<_>>().join(" "))
                > self.config.overlap_tokens
            {
                overlap_buffer.pop_front();
            }
        }

        // Don't forget the last chunk
        if !current_chunk.is_empty() {
            let token_count = estimate_tokens(&current_chunk);
            if token_count >= self.config.min_chunk_size {
                chunks.push(Chunk {
                    text: current_chunk.clone(),
                    char_range: (
                        base_pos + current_start,
                        base_pos + current_start + current_chunk.len(),
                    ),
                    token_count,
                    block_type,
                    section_path: section_path.to_vec(),
                    parent_heading: section_path.last().cloned(),
                });
            }
        }

        // If no chunks created (text too small), create one anyway
        if chunks.is_empty() && !text.is_empty() {
            chunks.push(Chunk {
                text: text.to_string(),
                char_range: (base_pos, base_pos + text.len()),
                token_count: estimate_tokens(text),
                block_type,
                section_path: section_path.to_vec(),
                parent_heading: section_path.last().cloned(),
            });
        }

        chunks
    }

    /// Calculate approximate character count for overlap tokens
    fn overlap_chars(&self) -> usize {
        // Assume ~4 chars per token
        self.config.overlap_tokens * 4
    }
}

/// Check if a line is a list item
fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    // Unordered list
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    // Ordered list (1. 2. etc)
    let mut chars = trimmed.chars();
    if let Some(first) = chars.next() {
        if first.is_ascii_digit() {
            // Check for more digits followed by . or )
            for c in chars {
                if c == '.' || c == ')' {
                    return true;
                } else if !c.is_ascii_digit() {
                    break;
                }
            }
        }
    }
    false
}

/// Estimate token count (rough approximation: ~4 chars per token)
pub fn estimate_tokens(text: &str) -> usize {
    // More accurate estimation: count words and add punctuation
    let words = text.split_whitespace().count();
    let punct = text.chars().filter(|c| c.is_ascii_punctuation()).count();
    words + punct / 4
}

/// Split text into sentences (simple heuristic)
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();

    for c in text.chars() {
        current.push(c);

        // End sentence on . ! ? followed by space or end
        if c == '.' || c == '!' || c == '?' {
            // Don't split on abbreviations like "pl." "etc."
            let trimmed = current.trim();
            if trimmed.len() > 3 {
                sentences.push(current.trim().to_string());
                current.clear();
            }
        }
    }

    if !current.trim().is_empty() {
        sentences.push(current.trim().to_string());
    }

    sentences
}

/// Extract tables from markdown text
pub fn extract_tables(text: &str) -> Vec<String> {
    let mut tables = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].trim().starts_with('|') && lines[i].contains('|') {
            let mut table = String::new();
            while i < lines.len() && lines[i].trim().starts_with('|') {
                table.push_str(lines[i]);
                table.push('\n');
                i += 1;
            }
            tables.push(table.trim_end().to_string());
        } else {
            i += 1;
        }
    }

    tables
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        let text = "Hello world, this is a test.";
        let tokens = estimate_tokens(text);
        assert!(tokens > 0);
        assert!(tokens < 20); // Should be around 6-8 tokens
    }

    #[test]
    fn test_is_list_item() {
        assert!(is_list_item("- item"));
        assert!(is_list_item("* item"));
        assert!(is_list_item("+ item"));
        assert!(is_list_item("1. item"));
        assert!(is_list_item("10. item"));
        assert!(is_list_item("  - indented"));
        assert!(!is_list_item("normal text"));
        assert!(!is_list_item("# heading"));
    }

    #[test]
    fn test_split_sentences() {
        let text = "First sentence. Second sentence! Third one?";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 3);
    }

    #[test]
    fn test_chunk_simple_markdown() {
        let md = r#"
# Introduction

This is the introduction paragraph.

## Details

More details here.
"#;

        let chunker = Chunker::with_defaults();
        let chunks = chunker.chunk_markdown(md);

        assert!(!chunks.is_empty());
        // Should have section path
        let detail_chunk = chunks.iter().find(|c| c.text.contains("More details"));
        assert!(detail_chunk.is_some());
    }

    #[test]
    fn test_preserve_table() {
        let md = r#"
# Table Section

| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
| Cell 3   | Cell 4   |

Text after table.
"#;

        let chunker = Chunker::with_defaults();
        let chunks = chunker.chunk_markdown(md);

        // Find table chunk
        let table_chunk = chunks.iter().find(|c| c.block_type == BlockType::Table);
        assert!(table_chunk.is_some());
        assert!(table_chunk.unwrap().text.contains("Header 1"));
        assert!(table_chunk.unwrap().text.contains("Cell 4"));
    }

    #[test]
    fn test_preserve_code_block() {
        let md = r#"
# Code Example

```rust
fn main() {
    println!("Hello");
}
```

Some text.
"#;

        let chunker = Chunker::with_defaults();
        let chunks = chunker.chunk_markdown(md);

        // Find code chunk
        let code_chunk = chunks.iter().find(|c| c.block_type == BlockType::Code);
        assert!(code_chunk.is_some());
        assert!(code_chunk.unwrap().text.contains("println"));
    }

    #[test]
    fn test_section_path() {
        let md = r#"
# Chapter 1

Intro text.

## Section 1.1

Section content.

### Subsection 1.1.1

Deep content.
"#;

        let chunker = Chunker::with_defaults();
        let chunks = chunker.chunk_markdown(md);

        // Find deep content chunk
        let deep_chunk = chunks.iter().find(|c| c.text.contains("Deep content"));
        assert!(deep_chunk.is_some());

        let path = &deep_chunk.unwrap().section_path;
        assert_eq!(path.len(), 3);
        assert!(path[0].contains("Chapter"));
        assert!(path[1].contains("Section"));
        assert!(path[2].contains("Subsection"));
    }

    #[test]
    fn test_extract_tables() {
        let md = r#"
Some text.

| A | B |
|---|---|
| 1 | 2 |

More text.

| C | D |
|---|---|
| 3 | 4 |
"#;

        let tables = extract_tables(md);
        assert_eq!(tables.len(), 2);
    }

    #[test]
    fn test_chunking_long_text() {
        // Generate long text
        let long_text = (0..500)
            .map(|i| format!("This is sentence number {}. ", i))
            .collect::<String>();

        let md = format!("# Long Document\n\n{}", long_text);

        let config = ChunkConfig {
            max_tokens: 100,
            overlap_tokens: 20,
            min_chunk_size: 10,
            ..Default::default()
        };

        let chunker = Chunker::new(config);
        let chunks = chunker.chunk_markdown(&md);

        // Should have multiple chunks
        assert!(chunks.len() > 1);

        // Each chunk should be within limits (approximately)
        for chunk in &chunks {
            assert!(chunk.token_count <= 150); // Some tolerance
        }
    }
}
