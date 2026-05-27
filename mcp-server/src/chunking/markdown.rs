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
    // Tracks whether the chunk boundary falls inside a fenced code block, so a
    // `# comment` line in code is not misdetected as a markdown heading.
    let mut fence_open = false;
    // Tracks the current table's "<header>\n<separator>" block so a chunk that
    // continues a table without its own separator gets the header prepended (#63).
    let mut current_table_header: Option<String> = None;

    // Create chunks with overlap applied
    for (index, raw_chunk) in raw_chunks.iter().enumerate() {
        let trimmed = raw_chunk.trim();
        let (start_byte, end_byte) = positions[index];

        // Detect heading from raw (non-overlapped) chunk text, but only when the
        // chunk does not start inside a fenced code block.
        if !fence_open {
            if let Some((heading, level)) = extract_heading(trimmed) {
                update_section_path(&mut section_path, &heading, level);
                current_heading = Some((heading, level));
                // A new section starts a fresh table context, so a later table's
                // data rows never inherit a previous table's header.
                current_table_header = None;
            }
        }

        // Update fence state from this chunk for the next iteration: an odd
        // number of fence markers toggles whether we end inside a code block.
        if count_fence_lines(raw_chunk) % 2 == 1 {
            fence_open = !fence_open;
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

        // Table header propagation (#63): keep table chunks self-interpretable.
        // Detect on the RAW (non-overlap) slice — the prepended overlap text would
        // otherwise hide that this chunk starts with table rows.
        let raw_slice = &content[start_byte..end_byte];
        let (final_text, propagated_header) =
            if let Some(header) = super::find_table_header(raw_slice) {
                // Chunk carries its own header → it becomes the current table header.
                current_table_header = Some(header);
                (chunk_text.to_string(), None)
            } else if raw_slice
                .lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| super::is_table_row(l) && !super::is_table_separator(l))
                .unwrap_or(false)
            {
                // Chunk starts with table data rows but has no separator → continuation:
                // prepend the tracked header and record it so the merge strips it exactly.
                match current_table_header.clone() {
                    Some(header) => (format!("{}\n{}", header, chunk_text), Some(header)),
                    None => (chunk_text.to_string(), None),
                }
            } else {
                // Non-table content ends the current table context.
                if !raw_slice.lines().any(super::is_table_row) {
                    current_table_header = None;
                }
                (chunk_text.to_string(), None)
            };

        let mut chunk = Chunk::new(index, total, final_text, start_char, end_char);

        // Add heading metadata
        if let Some((ref heading, level)) = current_heading {
            chunk = chunk.with_heading(heading.clone(), level);
        }

        if !section_path.is_empty() {
            chunk = chunk.with_section_path(section_path.clone());
        }

        if let Some(header) = propagated_header {
            chunk = chunk.with_table_header(header);
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

/// Count lines that open or close a fenced code block (``` or ~~~).
///
/// Each such line toggles the in-code-block state; an odd count means the
/// chunk flips the state for the following chunk.
fn count_fence_lines(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            t.starts_with("```") || t.starts_with("~~~")
        })
        .count()
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

    /// #63: a spurious blank line inside a markdown table (common in
    /// PDF-converted docs) must not strand the data rows in a chunk separate
    /// from their header/separator. With a chunk_size that comfortably fits the
    /// whole table, it should stay in a single chunk.
    #[test]
    fn test_table_blank_line_not_fragmented_issue63() {
        let content = "# Árlista\n\n\
            | Megnevezés | Ár |\n\
            |---|---|\n\
            | PEF-35 | 1750000 |\n\
            \n\
            | PEF-50 | 2100000 |\n\
            | PEF-70 | 2500000 |\n";
        // Forcing chunk_size so the table is split: every chunk that carries
        // table data rows must still expose the column header (separator row),
        // otherwise the values are uninterpretable in isolation. Tested with
        // BOTH overlap=0 and overlap>0 — the latter is the production default and
        // the case the first implementation silently failed (review finding).
        for overlap in [0usize, 20] {
            let chunks = split(content, 50, overlap).unwrap();
            for c in &chunks {
                if c.text.contains("PEF-") {
                    assert!(
                        c.text.contains("---") && c.text.contains("Megnevezés"),
                        "overlap={overlap}: table data chunk lacks its header:\n{}",
                        c.text
                    );
                }
            }
        }
    }

    /// #63 review: a later table under a new heading must not inherit the
    /// previous table's header (stale-header propagation would mislabel columns).
    #[test]
    fn test_table_header_not_stale_across_sections() {
        let content = "# T1\n\n\
            | A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\n\
            # T2\n\n\
            | C | D |\n|---|---|\n| 5 | 6 |\n| 7 | 8 |\n";
        for overlap in [0usize, 15] {
            let chunks = split(content, 40, overlap).unwrap();
            for c in &chunks {
                // Any chunk carrying table-2 data must never expose table-1's header.
                if c.text.contains("| 7 | 8 |") || c.text.contains("| 5 | 6 |") {
                    assert!(
                        !c.text.contains("| A | B |"),
                        "overlap={overlap}: table-2 chunk wrongly carries table-1 header:\n{}",
                        c.text
                    );
                }
            }
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

    #[test]
    fn test_count_fence_lines() {
        assert_eq!(count_fence_lines("no fences here"), 0);
        assert_eq!(count_fence_lines("```bash\necho hi\n```"), 2);
        assert_eq!(count_fence_lines("text\n~~~\ncode"), 1);
        assert_eq!(count_fence_lines("  ```rust\nfn main(){}\n  ```"), 2);
    }

    /// A `#`-prefixed line inside a fenced code block must not be treated as a
    /// markdown heading (would pollute section_path).
    #[test]
    fn test_heading_not_detected_inside_code_fence() {
        let content = "# Real Heading\n\nSome introductory text to fill space.\n\n\
                        ```bash\n# not a heading, just a shell comment\necho hello\n```\n\n\
                        More body text after the code block.";
        // Small chunk size forces the code comment toward a chunk boundary.
        let chunks = split(content, 40, 0).unwrap();
        for chunk in &chunks {
            if let Some(ref h) = chunk.heading {
                assert!(
                    !h.contains("not a heading"),
                    "code comment misdetected as heading: {}",
                    h
                );
            }
            if let Some(ref path) = chunk.section_path {
                assert!(
                    !path.iter().any(|s| s.contains("not a heading")),
                    "code comment leaked into section_path: {:?}",
                    path
                );
            }
        }
    }
}
