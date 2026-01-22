//! Plain text splitting with overlap

use crate::chunk::Chunk;
use crate::error::{GaploaderError, Result};
use text_splitter::TextSplitter;

/// Split plain text into chunks with overlap
///
/// OOM Protection: Uses try_reserve() before allocation (CLAUDE.md compliance)
pub fn split(
    content: &str,
    source_file: &str,
    filename: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<Vec<Chunk>> {
    // text-splitter uses a range for chunk size
    // The overlap is achieved by using a range where min = chunk_size - overlap
    let min_size = chunk_size.saturating_sub(overlap);
    let splitter = TextSplitter::new(min_size..chunk_size);

    // First pass: count chunks for try_reserve (CLAUDE.md: streaming count)
    let total = splitter.chunks(content).count();

    // OOM Protection: try_reserve before allocation (CLAUDE.md requirement)
    let mut chunks = Vec::new();
    chunks.try_reserve(total).map_err(|_| GaploaderError::OutOfMemory {
        requested: total,
        context: format!("text chunks for {}", filename),
    })?;

    let mut char_offset = 0;

    // Second pass: create chunks (iterator, no intermediate Vec)
    for (index, raw_chunk) in splitter.chunks(content).enumerate() {
        let start_char = char_offset;
        let end_char = start_char + raw_chunk.len();

        let chunk = Chunk::new(
            source_file,
            filename,
            raw_chunk.to_string(),
            index,
            total,
            start_char,
            end_char,
        );

        chunks.push(chunk);

        // Move offset, accounting for overlap
        // In text-splitter, chunks can overlap, so we find where this chunk
        // actually starts in the original content
        if let Some(pos) = content[char_offset..].find(raw_chunk) {
            char_offset += pos + raw_chunk.len();
            // Subtract overlap for next chunk
            if overlap > 0 && index < total - 1 {
                char_offset = char_offset.saturating_sub(overlap);
            }
        }
    }

    Ok(chunks)
}
