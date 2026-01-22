//! Load command implementation

use crate::config::Config;
use crate::error::{GaploaderError, Result};
use crate::loader::{self, MAX_FILE_SIZE_BYTES};
use crate::splitter;
use crate::SplitMode;
use std::path::PathBuf;

/// Run the load command
pub async fn run(
    config: &Config,
    file: PathBuf,
    collection: Option<String>,
    mode: SplitMode,
    chunk_size: usize,
    overlap: usize,
    embed: bool,
    provider: String,
    dry_run: bool,
    clear: bool,
) -> Result<()> {
    // Check file exists
    if !file.exists() {
        return Err(GaploaderError::FileNotFound(file));
    }

    // OOM Protection: Check file size
    let metadata = std::fs::metadata(&file)?;
    if metadata.len() > MAX_FILE_SIZE_BYTES {
        return Err(GaploaderError::FileTooLarge {
            path: file.clone(),
            size_mb: metadata.len() as f64 / (1024.0 * 1024.0),
            max_mb: MAX_FILE_SIZE_BYTES / (1024 * 1024),
        });
    }

    println!("Loading: {}", file.display());
    println!("  Mode: {:?}", mode.resolve(&file));
    println!("  Chunk size: {} chars", chunk_size);
    if mode.resolve(&file) == SplitMode::Text {
        println!("  Overlap: {} chars", overlap);
    }
    if embed {
        println!("  Embedding: {} (provider: {})", embed, provider);
    }

    if dry_run {
        // Just show preview
        println!();
        println!("=== DRY RUN - Preview chunks ===");
        return run_preview(&file, mode, chunk_size, overlap);
    }

    // Run actual load
    let result = loader::load_file(
        config,
        &file,
        collection.as_deref(),
        mode,
        chunk_size,
        overlap,
        embed,
        &provider,
        clear,
    )
    .await?;

    result.print_summary();

    Ok(())
}

/// Preview chunks (for dry-run)
fn run_preview(file: &PathBuf, mode: SplitMode, chunk_size: usize, overlap: usize) -> Result<()> {
    let content = std::fs::read_to_string(file)?;

    let filename = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let source_file = file.to_string_lossy().to_string();

    let resolved_mode = mode.resolve(file);

    let chunks = match resolved_mode {
        SplitMode::Markdown => {
            splitter::split_markdown(&content, &source_file, filename, chunk_size)?
        }
        SplitMode::Text | SplitMode::Auto => {
            splitter::split_text(&content, &source_file, filename, chunk_size, overlap)?
        }
    };

    println!("Total chunks: {}", chunks.len());
    println!();

    for (i, chunk) in chunks.iter().enumerate().take(10) {
        println!("--- Chunk {} ---", i);
        if let Some(ref heading) = chunk.heading {
            println!("Heading: {} (level {})", heading, chunk.heading_level.unwrap_or(0));
        }
        if let Some(ref path) = chunk.section_path {
            println!("Section: {}", path.join(" > "));
        }
        println!("Length: {} chars", chunk.content_length);
        println!();

        // Show first 200 chars
        let preview = if chunk.content.len() > 200 {
            format!("{}...", &chunk.content[..200])
        } else {
            chunk.content.clone()
        };
        println!("{}", preview);
        println!();
    }

    if chunks.len() > 10 {
        println!("... and {} more chunks", chunks.len() - 10);
    }

    Ok(())
}
