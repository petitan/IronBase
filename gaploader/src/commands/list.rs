//! List command - show loaded files

use crate::bridge::BridgeClient;
use crate::config::Config;
use crate::error::Result;
use crate::mcp_client::McpClient;

/// Run the list command
pub async fn run(
    config: &Config,
    collection: Option<String>,
    source: Option<String>,
) -> Result<()> {
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

    if let Some(coll) = collection {
        // List files in specific collection
        list_collection_files(&mut client, &coll, source.as_deref()).await?;
    } else {
        // List all collections that look like gaploader collections
        let collections = client.list_collections().await?;

        println!("Collections:");
        println!();

        for coll in &collections {
            // Check if collection has gaploader structure (source.file field)
            let count = client
                .count(&coll, &serde_json::json!({ "source.file": { "$exists": true } }))
                .await
                .unwrap_or(0);

            if count > 0 {
                println!("  \x1b[1m{}\x1b[0m ({} chunks)", coll, count);

                // Get distinct source files
                let files = client.distinct(&coll, "source.filename").await?;
                for file in files.iter().take(5) {
                    if let Some(filename) = file.as_str() {
                        println!("    - {}", filename);
                    }
                }
                if files.len() > 5 {
                    println!("    ... and {} more files", files.len() - 5);
                }
                println!();
            }
        }
    }

    client.close().await?;
    Ok(())
}

/// List files in a specific collection
async fn list_collection_files(
    client: &mut McpClient,
    collection: &str,
    source_filter: Option<&str>,
) -> Result<()> {
    println!("Collection: {}", collection);
    println!();

    // Build filter
    let filter = if let Some(pattern) = source_filter {
        serde_json::json!({
            "source.filename": { "$regex": pattern }
        })
    } else {
        serde_json::json!({
            "source.file": { "$exists": true }
        })
    };

    // Get distinct source files
    let files = client.distinct(collection, "source.filename").await?;

    if files.is_empty() {
        println!("No gaploader chunks found in this collection.");
        return Ok(());
    }

    println!("Source files:");

    for file in &files {
        if let Some(filename) = file.as_str() {
            // Count chunks for this file
            let file_filter = serde_json::json!({ "source.filename": filename });
            let chunk_count = client.count(collection, &file_filter).await?;

            // Check if embedded
            let embed_filter = serde_json::json!({
                "source.filename": filename,
                "embedding": { "$exists": true }
            });
            let embed_count = client.count(collection, &embed_filter).await?;

            let embed_status = if embed_count > 0 {
                format!(" \x1b[32m[embedded]\x1b[0m")
            } else {
                String::new()
            };

            println!("  {} ({} chunks){}", filename, chunk_count, embed_status);
        }
    }

    // Total stats
    let total = client.count(collection, &filter).await?;
    println!();
    println!("Total: {} chunks from {} files", total, files.len());

    Ok(())
}
