use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ironbase_core::{storage::StorageEngine, DatabaseCore};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "ironbase")]
#[command(about = "IronBase CLI - Command-line interface for IronBase database")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Import data from JSON file into database
    Import {
        /// JSON file to import
        file: PathBuf,
        /// Database file path
        #[arg(long, default_value = "ironbase.mlite")]
        db: PathBuf,
    },
    /// Export database to JSON file
    Export {
        /// Output JSON file
        file: PathBuf,
        /// Database file path
        #[arg(long, default_value = "ironbase.mlite")]
        db: PathBuf,
        /// Export only specific collection
        #[arg(long)]
        collection: Option<String>,
    },
    /// Schema management commands
    Schema {
        #[command(subcommand)]
        action: SchemaAction,
    },
}

#[derive(Subcommand)]
enum SchemaAction {
    /// Load schema from JSON file or directory (modular)
    Load {
        /// Schema file (.json) or directory containing *.schema.json files
        path: PathBuf,
        /// Database file path
        #[arg(long, default_value = "ironbase.mlite")]
        db: PathBuf,
        /// Collection name (required for single file, ignored for directory)
        #[arg(long)]
        collection: Option<String>,
    },
    /// Save schema to JSON file or directory
    Save {
        /// Output file (.json) or directory
        path: PathBuf,
        /// Database file path
        #[arg(long, default_value = "ironbase.mlite")]
        db: PathBuf,
        /// Collection name (for single file export)
        #[arg(long)]
        collection: Option<String>,
        /// Export all schemas (for directory export)
        #[arg(long)]
        all: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Import { file, db } => import_data(&file, &db),
        Commands::Export {
            file,
            db,
            collection,
        } => export_data(&file, &db, collection.as_deref()),
        Commands::Schema { action } => match action {
            SchemaAction::Load {
                path,
                db,
                collection,
            } => load_schema(&path, &db, collection.as_deref()),
            SchemaAction::Save {
                path,
                db,
                collection,
                all,
            } => save_schema(&path, &db, collection.as_deref(), all),
        },
    }
}

/// Import data from JSON file
/// Format: { "collection_name": [documents...], ... }
fn import_data(file: &Path, db_path: &Path) -> Result<()> {
    let content = fs::read_to_string(file)
        .with_context(|| format!("Failed to read file: {}", file.display()))?;

    let data: Map<String, Value> = serde_json::from_str(&content)
        .with_context(|| format!("Invalid JSON in file: {}", file.display()))?;

    let db = DatabaseCore::<StorageEngine>::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    let mut total_docs = 0;

    for (collection_name, documents) in data {
        let docs = documents
            .as_array()
            .with_context(|| format!("Collection '{}' must be an array", collection_name))?;

        for doc in docs {
            let doc_map: HashMap<String, Value> = doc
                .as_object()
                .with_context(|| "Document must be an object")?
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            db.insert_one(&collection_name, doc_map)
                .with_context(|| format!("Failed to insert document into {}", collection_name))?;
            total_docs += 1;
        }

        println!(
            "Imported {} documents into '{}'",
            docs.len(),
            collection_name
        );
    }

    println!(
        "Total: {} documents imported to {}",
        total_docs,
        db_path.display()
    );
    Ok(())
}

/// Export database to JSON file
fn export_data(file: &Path, db_path: &Path, collection_filter: Option<&str>) -> Result<()> {
    let db = DatabaseCore::<StorageEngine>::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    let collections = db.list_collections();

    let mut output: Map<String, Value> = Map::new();
    let mut total_docs = 0;

    for coll_name in collections {
        if let Some(filter) = collection_filter {
            if coll_name != filter {
                continue;
            }
        }

        let coll = db
            .collection(&coll_name)
            .with_context(|| format!("Failed to get collection: {}", coll_name))?;

        let docs = coll
            .find(&serde_json::json!({}))
            .with_context(|| format!("Failed to query collection: {}", coll_name))?;

        println!("Exporting {} documents from '{}'", docs.len(), coll_name);
        total_docs += docs.len();
        output.insert(coll_name.clone(), Value::Array(docs));
    }

    let json =
        serde_json::to_string_pretty(&output).with_context(|| "Failed to serialize to JSON")?;

    fs::write(file, json)
        .with_context(|| format!("Failed to write to file: {}", file.display()))?;

    println!(
        "Total: {} documents exported to {}",
        total_docs,
        file.display()
    );
    Ok(())
}

/// Load schema from file or directory (modular)
fn load_schema(path: &Path, db_path: &Path, collection: Option<&str>) -> Result<()> {
    let db = DatabaseCore::<StorageEngine>::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    if path.is_dir() {
        // Modular: load all *.schema.json files from directory
        let entries = fs::read_dir(path)
            .with_context(|| format!("Failed to read directory: {}", path.display()))?;

        let mut count = 0;
        for entry in entries {
            let entry = entry?;
            let file_path = entry.path();

            if let Some(name) = file_path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".schema.json") {
                    let coll_name = name.trim_end_matches(".schema.json");
                    let schema_content = fs::read_to_string(&file_path)
                        .with_context(|| format!("Failed to read: {}", file_path.display()))?;

                    let schema: Value = serde_json::from_str(&schema_content)
                        .with_context(|| format!("Invalid JSON in: {}", file_path.display()))?;

                    let coll = db
                        .collection(coll_name)
                        .with_context(|| format!("Failed to get collection: {}", coll_name))?;

                    coll.set_schema(Some(schema))
                        .with_context(|| format!("Failed to set schema for: {}", coll_name))?;

                    println!("Loaded schema for '{}'", coll_name);
                    count += 1;
                }
            }
        }

        println!("Total: {} schemas loaded from {}", count, path.display());
    } else {
        // Single file: require collection name
        let coll_name = collection.ok_or_else(|| {
            anyhow::anyhow!("--collection required when loading single schema file")
        })?;

        let schema_content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read: {}", path.display()))?;

        let schema: Value = serde_json::from_str(&schema_content)
            .with_context(|| format!("Invalid JSON in: {}", path.display()))?;

        let coll = db
            .collection(coll_name)
            .with_context(|| format!("Failed to get collection: {}", coll_name))?;

        coll.set_schema(Some(schema))
            .with_context(|| format!("Failed to set schema for: {}", coll_name))?;

        println!("Loaded schema for '{}' from {}", coll_name, path.display());
    }

    Ok(())
}

/// Save schema to file or directory
fn save_schema(path: &Path, db_path: &Path, collection: Option<&str>, all: bool) -> Result<()> {
    let db = DatabaseCore::<StorageEngine>::open(db_path)
        .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

    if all
        || path.is_dir()
        || (collection.is_none() && path.extension().is_none_or(|e| e != "json"))
    {
        // Export all schemas to directory
        let dir_path = if path.exists() && path.is_dir() {
            path.to_path_buf()
        } else {
            fs::create_dir_all(path)
                .with_context(|| format!("Failed to create directory: {}", path.display()))?;
            path.to_path_buf()
        };

        let collections = db.list_collections();

        let mut count = 0;
        for coll_name in collections {
            let coll = db
                .collection(&coll_name)
                .with_context(|| format!("Failed to get collection: {}", coll_name))?;

            if let Some(schema) = coll.get_schema() {
                let file_path = dir_path.join(format!("{}.schema.json", coll_name));
                let json = serde_json::to_string_pretty(&schema)
                    .with_context(|| "Failed to serialize schema")?;

                fs::write(&file_path, json)
                    .with_context(|| format!("Failed to write: {}", file_path.display()))?;

                println!(
                    "Saved schema for '{}' to {}",
                    coll_name,
                    file_path.display()
                );
                count += 1;
            }
        }

        println!("Total: {} schemas saved to {}", count, dir_path.display());
    } else {
        // Single file export
        let coll_name = collection.ok_or_else(|| {
            anyhow::anyhow!("--collection required when saving single schema file")
        })?;

        let coll = db
            .collection(coll_name)
            .with_context(|| format!("Failed to get collection: {}", coll_name))?;

        let schema = coll
            .get_schema()
            .ok_or_else(|| anyhow::anyhow!("Collection '{}' has no schema", coll_name))?;

        let json =
            serde_json::to_string_pretty(&schema).with_context(|| "Failed to serialize schema")?;

        fs::write(path, json).with_context(|| format!("Failed to write: {}", path.display()))?;

        println!("Saved schema for '{}' to {}", coll_name, path.display());
    }

    Ok(())
}
