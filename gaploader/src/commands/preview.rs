//! Preview command - show chunks without loading

use crate::error::Result;
use crate::splitter;
use crate::SplitMode;
use std::path::PathBuf;

/// Run the preview command
pub fn run(
    file: PathBuf,
    mode: SplitMode,
    chunk_size: usize,
    overlap: usize,
    limit: usize,
) -> Result<()> {
    // Check file exists
    if !file.exists() {
        return Err(crate::error::GaploaderError::FileNotFound(file));
    }

    let content = std::fs::read_to_string(&file)?;

    let filename = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let source_file = file.to_string_lossy().to_string();

    let resolved_mode = mode.resolve(&file);

    println!("File: {}", file.display());
    println!("Mode: {:?}", resolved_mode);
    println!("Chunk size: {} chars", chunk_size);
    if resolved_mode == SplitMode::Text {
        println!("Overlap: {} chars", overlap);
    }
    println!();

    let chunks = match resolved_mode {
        SplitMode::Markdown => {
            splitter::split_markdown(&content, &source_file, filename, chunk_size)?
        }
        SplitMode::Text | SplitMode::Auto => {
            splitter::split_text(&content, &source_file, filename, chunk_size, overlap)?
        }
    };

    let total = chunks.len();
    println!("Total chunks: {}", total);
    println!("Showing first {} chunks:", limit.min(total));
    println!();

    for (i, chunk) in chunks.iter().enumerate().take(limit) {
        println!("\x1b[1m=== Chunk {} / {} ===\x1b[0m", i + 1, total);
        println!(
            "\x1b[90mPosition: chars {}-{}\x1b[0m",
            chunk.chunk.start_char, chunk.chunk.end_char
        );

        if let Some(ref heading) = chunk.heading {
            println!(
                "\x1b[36mHeading (h{}): {}\x1b[0m",
                chunk.heading_level.unwrap_or(0),
                heading
            );
        }

        if let Some(ref path) = chunk.section_path {
            println!("\x1b[33mSection: {}\x1b[0m", path.join(" > "));
        }

        println!("\x1b[90mLength: {} chars\x1b[0m", chunk.content_length);
        println!();

        // Show content with truncation
        let max_preview = 500;
        let preview = if chunk.content.len() > max_preview {
            format!(
                "{}\n\x1b[90m... ({} more chars)\x1b[0m",
                &chunk.content[..max_preview],
                chunk.content.len() - max_preview
            )
        } else {
            chunk.content.clone()
        };

        println!("{}", preview);
        println!();
    }

    if total > limit {
        println!(
            "\x1b[90m... {} more chunks (use --limit to see more)\x1b[0m",
            total - limit
        );
    }

    Ok(())
}
