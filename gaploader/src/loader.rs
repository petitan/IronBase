//! Loader orchestration - chunk, insert, and optionally embed

use crate::bridge::BridgeClient;
use crate::chunk::Chunk;
use crate::config::Config;
use crate::error::{GaploaderError, Result};
use crate::mcp_client::McpClient;
use crate::splitter;
use crate::SplitMode;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;

/// Batch size for insert_many
const INSERT_BATCH_SIZE: usize = 100;

/// Batch size for embed_batch
const EMBED_BATCH_SIZE: usize = 50;

/// Maximum file size in bytes (100 MB) - OOM protection
pub const MAX_FILE_SIZE_BYTES: u64 = 100 * 1024 * 1024;

/// Load a file into IronBase
pub async fn load_file(
    config: &Config,
    file_path: &Path,
    collection: Option<&str>,
    mode: SplitMode,
    chunk_size: usize,
    overlap: usize,
    embed: bool,
    provider: &str,
    clear: bool,
) -> Result<LoadResult> {
    // OOM Protection: Check file size before reading
    let metadata = std::fs::metadata(file_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GaploaderError::FileNotFound(file_path.to_path_buf())
        } else {
            GaploaderError::Io(e)
        }
    })?;

    if metadata.len() > MAX_FILE_SIZE_BYTES {
        return Err(GaploaderError::FileTooLarge {
            path: file_path.to_path_buf(),
            size_mb: metadata.len() as f64 / (1024.0 * 1024.0),
            max_mb: MAX_FILE_SIZE_BYTES / (1024 * 1024),
        });
    }

    // Read file (safe after size check)
    let content = std::fs::read_to_string(file_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GaploaderError::FileNotFound(file_path.to_path_buf())
        } else {
            GaploaderError::Io(e)
        }
    })?;

    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let source_file = file_path.to_string_lossy().to_string();

    // Determine collection name
    let collection_name = collection
        .map(String::from)
        .unwrap_or_else(|| config.collection_name(filename));

    // Resolve split mode
    let resolved_mode = mode.resolve(file_path);

    // Split content into chunks
    let chunks = match resolved_mode {
        SplitMode::Markdown => {
            splitter::split_markdown(&content, &source_file, filename, chunk_size)?
        }
        SplitMode::Text | SplitMode::Auto => {
            splitter::split_text(&content, &source_file, filename, chunk_size, overlap)?
        }
    };

    let total_chunks = chunks.len();

    if total_chunks == 0 {
        return Ok(LoadResult {
            collection: collection_name,
            chunks_created: 0,
            chunks_embedded: 0,
            mode: resolved_mode,
        });
    }

    // Connect to MCP via bridge
    let bridge_path = config.find_bridge_path()?;
    let bridge = BridgeClient::spawn(
        &bridge_path,
        &config.bridge.server_url,
        config.bridge.api_key.as_deref(),
        config.bridge.insecure,
    )
    .await?;

    let mut client = McpClient::new(bridge);

    // Create collection if needed
    if !client.collection_exists(&collection_name).await? {
        client.create_collection(&collection_name).await?;
        println!("Created collection: {}", collection_name);
    }

    // Clear collection if requested
    if clear {
        let deleted = client
            .delete_many(&collection_name, &serde_json::json!({}))
            .await?;
        println!("Cleared {} documents from {}", deleted, collection_name);
    }

    // Insert chunks in batches
    let pb = ProgressBar::new(total_chunks as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} chunks")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut chunks_created = 0;

    for batch in chunks.chunks(INSERT_BATCH_SIZE) {
        let docs: Vec<serde_json::Value> = batch.iter().map(|c| c.to_value()).collect();
        let _ids = client.insert_many(&collection_name, &docs).await?;
        chunks_created += batch.len();
        pb.set_position(chunks_created as u64);
    }

    pb.finish_with_message("Chunks inserted");

    // Create indexes
    client.create_index(&collection_name, "source.file").await?;
    client.create_index(&collection_name, "chunk.index").await?;
    println!("Created indexes on source.file and chunk.index");

    // Embed if requested
    let chunks_embedded = if embed {
        embed_chunks(&mut client, &collection_name, &chunks, provider).await?
    } else {
        0
    };

    // Close connection
    client.close().await?;

    Ok(LoadResult {
        collection: collection_name,
        chunks_created,
        chunks_embedded,
        mode: resolved_mode,
    })
}

/// Embed chunks and update documents
async fn embed_chunks(
    client: &mut McpClient,
    collection: &str,
    chunks: &[Chunk],
    provider: &str,
) -> Result<usize> {
    let pb = ProgressBar::new(chunks.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.yellow/red}] {pos}/{len} embeddings")
            .unwrap()
            .progress_chars("#>-"),
    );

    let mut embedded = 0;
    let mut embedding_dim: Option<usize> = None;

    for batch in chunks.chunks(EMBED_BATCH_SIZE) {
        // Extract text references (no cloning - OOM protection)
        let texts: Vec<&str> = batch.iter().map(|c| c.content.as_str()).collect();

        // Get embeddings
        let embeddings = client.embed_batch(&texts, Some(provider)).await?;

        if embeddings.len() != batch.len() {
            return Err(GaploaderError::EmbeddingError(format!(
                "Embedding count mismatch: expected {}, got {}",
                batch.len(),
                embeddings.len()
            )));
        }

        // Store dimension from first embedding
        if embedding_dim.is_none() {
            if let Some(first) = embeddings.first() {
                embedding_dim = Some(first.len());
            }
        }

        // Update documents with embeddings
        for (chunk, embedding) in batch.iter().zip(embeddings.iter()) {
            let filter = serde_json::json!({ "_id": chunk._id });
            let update = serde_json::json!({
                "$set": {
                    "embedding": embedding,
                    "embedding_provider": provider,
                    "embedding_dim": embedding.len()
                }
            });

            client.update_many(collection, &filter, &update).await?;
            embedded += 1;
            pb.set_position(embedded as u64);
        }
    }

    pb.finish_with_message("Embeddings added");

    // Create vector index with the detected dimension
    if let Some(dimension) = embedding_dim {
        if dimension > 0 {
            client
                .create_vector_index(collection, "embedding", dimension, "cosine")
                .await?;
            println!("Created vector index on embedding (dim={})", dimension);
        }
    }

    Ok(embedded)
}

/// Result of a load operation
pub struct LoadResult {
    pub collection: String,
    pub chunks_created: usize,
    pub chunks_embedded: usize,
    pub mode: SplitMode,
}

impl LoadResult {
    pub fn print_summary(&self) {
        println!();
        println!("Load complete:");
        println!("  Collection: {}", self.collection);
        println!("  Chunks: {}", self.chunks_created);
        println!(
            "  Mode: {}",
            match self.mode {
                SplitMode::Markdown => "markdown",
                SplitMode::Text => "text",
                SplitMode::Auto => "auto",
            }
        );
        if self.chunks_embedded > 0 {
            println!("  Embedded: {}", self.chunks_embedded);
        }
    }
}
