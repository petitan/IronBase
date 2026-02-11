//! Plain text splitting with configurable overlap
//!
//! Splits text into chunks with overlap to preserve context across chunk boundaries.

use super::{byte_to_char_offset, safe_byte_offset, Chunk, ChunkError};
use text_splitter::TextSplitter;

/// Split plain text into chunks with overlap
///
/// Uses `text-splitter` crate for natural boundary detection (sentences, words),
/// then applies overlap by extending each chunk backwards to include text from
/// the previous chunk.
pub fn split(content: &str, chunk_size: usize, overlap: usize) -> Result<Vec<Chunk>, ChunkError> {
    let splitter = TextSplitter::new(chunk_size);

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

    // Find actual byte positions of each chunk in the original content
    let mut positions: Vec<(usize, usize)> = Vec::with_capacity(total);
    let mut search_from: usize = 0;
    for raw_chunk in &raw_chunks {
        let safe_from = safe_byte_offset(content, search_from);
        let start = if let Some(pos) = content[safe_from..].find(raw_chunk) {
            safe_from + pos
        } else {
            safe_from
        };
        let end = (start + raw_chunk.len()).min(content.len());
        positions.push((start, end));
        search_from = end;
    }

    // Create chunks with overlap applied
    for (index, _raw_chunk) in raw_chunks.iter().enumerate() {
        let (start_byte, end_byte) = positions[index];

        // Apply overlap: extend backwards for chunks after the first
        let overlap_start = if index > 0 && overlap > 0 {
            super::overlap_start_byte(content, start_byte, overlap)
        } else {
            start_byte
        };

        let chunk_text = &content[overlap_start..end_byte];
        let start_char = byte_to_char_offset(content, overlap_start);
        let end_char = byte_to_char_offset(content, end_byte);

        let chunk = Chunk::new(index, total, chunk_text.to_string(), start_char, end_char);
        chunks.push(chunk);
    }

    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_text() {
        let content = "This is a test. It has multiple sentences. Each one is separate.";
        let chunks = split(content, 30, 10).unwrap();

        assert!(!chunks.is_empty());
        // Verify total count is set correctly
        for chunk in &chunks {
            assert_eq!(chunk.total, chunks.len());
        }
    }

    #[test]
    fn test_split_empty() {
        let chunks = split("", 100, 10).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_split_small() {
        let content = "Short text";
        let chunks = split(content, 100, 10).unwrap();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, content);
    }
}
