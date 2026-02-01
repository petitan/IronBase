//! Plain text splitting with configurable overlap
//!
//! Splits text into chunks with overlap to preserve context across chunk boundaries.

use super::{Chunk, ChunkError};
use text_splitter::TextSplitter;

/// Convert byte offset to character offset in a UTF-8 string
fn byte_to_char_offset(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset.min(content.len())].chars().count()
}

/// Split plain text into chunks with overlap
pub fn split(content: &str, chunk_size: usize, overlap: usize) -> Result<Vec<Chunk>, ChunkError> {
    // text-splitter uses a range for chunk size
    // The overlap is achieved by using a range where min = chunk_size - overlap
    let min_size = chunk_size.saturating_sub(overlap);
    let splitter = TextSplitter::new(min_size..chunk_size);

    // First pass: count chunks for allocation
    let total = splitter.chunks(content).count();

    if total == 0 {
        return Ok(vec![]);
    }

    // Pre-allocate with try_reserve for OOM protection
    let mut chunks = Vec::new();
    chunks
        .try_reserve(total)
        .map_err(|_| ChunkError::OutOfMemory { count: total })?;

    let mut byte_offset = 0;

    // Second pass: create chunks
    for (index, raw_chunk) in splitter.chunks(content).enumerate() {
        // Calculate byte offsets first
        let start_byte = byte_offset;
        let end_byte = start_byte + raw_chunk.len();

        // Convert byte offsets to character offsets for proper UTF-8 handling
        let start_char = byte_to_char_offset(content, start_byte);
        let end_char = byte_to_char_offset(content, end_byte);

        let chunk = Chunk::new(index, total, raw_chunk.to_string(), start_char, end_char);
        chunks.push(chunk);

        // Move offset, accounting for overlap
        // In text-splitter, chunks can overlap, so we find where this chunk
        // actually starts in the original content
        if let Some(pos) = content[byte_offset..].find(raw_chunk) {
            byte_offset += pos + raw_chunk.len();
            // Subtract overlap for next chunk (in bytes)
            if overlap > 0 && index < total - 1 {
                byte_offset = byte_offset.saturating_sub(overlap);
            }
        }
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
