//! Plain text splitting with overlap

use crate::chunk::Chunk;
use crate::error::Result;
use text_splitter::TextSplitter;

/// Split plain text into chunks with overlap
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

    let raw_chunks: Vec<&str> = splitter.chunks(content).collect();
    let total = raw_chunks.len();

    let mut chunks = Vec::with_capacity(total);
    let mut char_offset = 0;

    for (index, raw_chunk) in raw_chunks.into_iter().enumerate() {
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
