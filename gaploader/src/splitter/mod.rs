//! Text splitting module

pub mod markdown;
pub mod text;

use crate::chunk::Chunk;
use crate::error::Result;

/// Split text into chunks
pub fn split_text(
    content: &str,
    source_file: &str,
    filename: &str,
    chunk_size: usize,
    overlap: usize,
) -> Result<Vec<Chunk>> {
    text::split(content, source_file, filename, chunk_size, overlap)
}

/// Split markdown into chunks (at headings, no overlap)
pub fn split_markdown(
    content: &str,
    source_file: &str,
    filename: &str,
    chunk_size: usize,
) -> Result<Vec<Chunk>> {
    markdown::split(content, source_file, filename, chunk_size)
}
