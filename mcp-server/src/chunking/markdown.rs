//! Markdown-aware text splitting with configurable overlap
//!
//! Splits markdown content at structural boundaries (headings) while
//! preserving heading hierarchy and section context.

use super::{byte_to_char_offset, Chunk, ChunkError};
use text_splitter::MarkdownSplitter;

/// Split markdown content into chunks with overlap
///
/// Respects markdown structure and preserves heading hierarchy.
/// Overlap is applied by extending each chunk backwards to include text
/// from the previous chunk. Heading detection uses the raw (non-overlapped)
/// chunk text to maintain correct section path tracking.
pub fn split(content: &str, chunk_size: usize, overlap: usize) -> Result<Vec<Chunk>, ChunkError> {
    let splitter = MarkdownSplitter::new(chunk_size);

    // Collect non-overlapping chunks (string slices into content, minimal memory)
    let raw_chunks: Vec<&str> = splitter.chunks(content).collect();
    let total = raw_chunks.len();

    if total == 0 {
        return Ok(vec![]);
    }

    // Pre-allocate with try_reserve for OOM protection
    let mut chunks = Vec::new();
    chunks
        .try_reserve(total)
        .map_err(|_| ChunkError::OutOfMemory { count: total })?;

    // Compute byte positions using pointer arithmetic.
    // MarkdownSplitter::chunks() returns &str slices into the original content,
    // so the pointer difference gives the exact byte offset — always a valid
    // UTF-8 char boundary, no find() needed.
    let content_start = content.as_ptr() as usize;
    let mut positions: Vec<(usize, usize)> = Vec::with_capacity(total);
    for raw_chunk in &raw_chunks {
        let start = raw_chunk.as_ptr() as usize - content_start;
        let end = start + raw_chunk.len();
        debug_assert!(
            content.is_char_boundary(start),
            "start {} not a char boundary",
            start
        );
        debug_assert!(
            content.is_char_boundary(end),
            "end {} not a char boundary",
            end
        );
        positions.push((start, end));
    }

    let mut section_path: Vec<String> = Vec::new();
    let mut current_heading: Option<(String, u8)> = None;

    // Create chunks with overlap applied
    for (index, raw_chunk) in raw_chunks.iter().enumerate() {
        let trimmed = raw_chunk.trim();
        let (start_byte, end_byte) = positions[index];

        // Detect heading from raw (non-overlapped) chunk text
        if let Some((heading, level)) = extract_heading(trimmed) {
            update_section_path(&mut section_path, &heading, level);
            current_heading = Some((heading, level));
        }

        // Apply overlap: extend backwards for chunks after the first
        let overlap_start = if index > 0 && overlap > 0 {
            super::overlap_start_byte(content, start_byte, overlap)
        } else {
            start_byte
        };

        let chunk_text = &content[overlap_start..end_byte];
        let start_char = byte_to_char_offset(content, overlap_start);
        let end_char = byte_to_char_offset(content, end_byte);

        let mut chunk = Chunk::new(index, total, chunk_text.to_string(), start_char, end_char);

        // Add heading metadata
        if let Some((ref heading, level)) = current_heading {
            chunk = chunk.with_heading(heading.clone(), level);
        }

        if !section_path.is_empty() {
            chunk = chunk.with_section_path(section_path.clone());
        }

        chunks.push(chunk);
    }

    Ok(chunks)
}

/// Extract heading text and level from markdown heading line
fn extract_heading(text: &str) -> Option<(String, u8)> {
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim();

    // Count leading # characters
    let hash_count = trimmed.chars().take_while(|c| *c == '#').count();

    if (1..=6).contains(&hash_count) {
        let heading_text = trimmed[hash_count..].trim().to_string();
        if !heading_text.is_empty() {
            return Some((heading_text, hash_count as u8));
        }
    }

    None
}

/// Update section path based on new heading
fn update_section_path(path: &mut Vec<String>, heading: &str, level: u8) {
    let level = level as usize;

    // Truncate path to one level above current heading
    while path.len() >= level {
        path.pop();
    }

    // Add current heading
    path.push(heading.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_heading() {
        assert_eq!(extract_heading("# Title"), Some(("Title".to_string(), 1)));
        assert_eq!(
            extract_heading("## Section"),
            Some(("Section".to_string(), 2))
        );
        assert_eq!(
            extract_heading("### Subsection"),
            Some(("Subsection".to_string(), 3))
        );
        assert_eq!(extract_heading("Not a heading"), None);
        assert_eq!(extract_heading("####### Too many"), None);
    }

    #[test]
    fn test_section_path() {
        let mut path = Vec::new();

        update_section_path(&mut path, "Chapter 1", 1);
        assert_eq!(path, vec!["Chapter 1"]);

        update_section_path(&mut path, "Section 1.1", 2);
        assert_eq!(path, vec!["Chapter 1", "Section 1.1"]);

        update_section_path(&mut path, "Section 1.2", 2);
        assert_eq!(path, vec!["Chapter 1", "Section 1.2"]);

        update_section_path(&mut path, "Chapter 2", 1);
        assert_eq!(path, vec!["Chapter 2"]);
    }

    /// Regression test for issue #46: panic on UTF-8 multi-byte char boundary
    /// when chunk split falls inside 'ö', 'ő', 'ü', etc.
    #[test]
    fn test_split_markdown_utf8_boundary_issue46() {
        // Build content where chunk boundary is likely to fall inside multi-byte chars.
        // Hungarian text with many 2-byte chars (á é í ó ö ő ú ü ű).
        let section = "Összefüggő szöveg, amelyben rengeteg ékezetes karakter található: \
                        árvíztűrő tükörfúrógép, bögrécske, gyönyörű, különféle, \
                        süvöltöző szélvihar az óperenciás tengeren túl. ";
        // Repeat to create ~5000 chars (well above chunk_size=1000)
        let mut content = String::from("# Ékezetes fejezet\n\n");
        for i in 0..50 {
            if i % 10 == 0 && i > 0 {
                content.push_str(&format!("\n\n## Alfejezet {}\n\n", i));
            }
            content.push_str(section);
            content.push('\n');
        }
        // Add special chars from issue: ' (U+2019), ➢ (U+27A2), NBSP
        content
            .push_str("\n\nSpeciális: \u{2019}idézet\u{2019}, \u{27A2} nyíl, szóköz\u{a0}itt.\n");

        let result = split(&content, 1000, 100);
        assert!(
            result.is_ok(),
            "UTF-8 markdown chunking panicked (issue #46): {:?}",
            result.err()
        );
        let chunks = result.unwrap();
        assert!(
            chunks.len() > 1,
            "Expected multiple chunks, got {}",
            chunks.len()
        );

        // Verify all chunks are valid UTF-8 (they are Strings, so this is guaranteed,
        // but verify they contain actual content)
        for chunk in &chunks {
            assert!(
                !chunk.text.is_empty(),
                "Empty chunk at index {}",
                chunk.index
            );
        }
    }

    #[test]
    fn test_split_markdown() {
        let content = "# Title\n\nSome content here.\n\n## Section 1\n\nMore content.";
        let chunks = split(content, 100, 0).unwrap();

        assert!(!chunks.is_empty());
        // Verify total count is set correctly
        for chunk in &chunks {
            assert_eq!(chunk.total, chunks.len());
        }
    }
}
