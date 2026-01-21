//! Markdown splitting at structural boundaries (headings)

use crate::chunk::Chunk;
use crate::error::Result;
use text_splitter::MarkdownSplitter;

/// Split markdown at headings, no overlap
pub fn split(
    content: &str,
    source_file: &str,
    filename: &str,
    chunk_size: usize,
) -> Result<Vec<Chunk>> {
    // MarkdownSplitter respects markdown structure
    let splitter = MarkdownSplitter::new(chunk_size);

    let raw_chunks: Vec<&str> = splitter.chunks(content).collect();
    let total = raw_chunks.len();

    let mut chunks = Vec::with_capacity(total);
    let mut char_offset = 0;
    let mut section_path: Vec<String> = Vec::new();
    let mut current_heading: Option<(String, u8)> = None;

    for (index, raw_chunk) in raw_chunks.into_iter().enumerate() {
        let trimmed = raw_chunk.trim();

        // Detect heading at start of chunk
        if let Some((heading, level)) = extract_heading(trimmed) {
            // Update section path based on heading level
            update_section_path(&mut section_path, &heading, level);
            current_heading = Some((heading, level));
        }

        let start_char = if let Some(pos) = content[char_offset..].find(trimmed) {
            char_offset + pos
        } else {
            char_offset
        };
        let end_char = start_char + raw_chunk.len();

        let mut chunk = Chunk::new(
            source_file,
            filename,
            raw_chunk.to_string(),
            index,
            total,
            start_char,
            end_char,
        );

        // Add heading metadata
        if let Some((ref heading, level)) = current_heading {
            chunk = chunk.with_heading(heading.clone(), level);
        }

        if !section_path.is_empty() {
            chunk = chunk.with_section_path(section_path.clone());
        }

        chunks.push(chunk);
        char_offset = end_char;
    }

    Ok(chunks)
}

/// Extract heading text and level from markdown heading line
fn extract_heading(text: &str) -> Option<(String, u8)> {
    let first_line = text.lines().next()?;
    let trimmed = first_line.trim();

    // Count leading # characters
    let hash_count = trimmed.chars().take_while(|c| *c == '#').count();

    if hash_count >= 1 && hash_count <= 6 {
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
        assert_eq!(
            extract_heading("# Title"),
            Some(("Title".to_string(), 1))
        );
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
}
