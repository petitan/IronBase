use ironbase_core::{DatabaseCore, StorageEngine};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn usage() {
    eprintln!(
        "Usage: cargo run --example strip_level -- \\
         --db PATH --collection NAME --document FILE --output FILE [--schema FILE]"
    );
}

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn remove_level(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("level");
            for v in map.values_mut() {
                remove_level(v);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                remove_level(item);
            }
        }
        _ => {}
    }
}

fn value_to_map(value: Value) -> Result<HashMap<String, Value>, String> {
    value
        .as_object()
        .cloned()
        .map(|m| m.into_iter().collect())
        .ok_or_else(|| "Document root must be a JSON object".to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a == "--help") || args.len() < 9 {
        usage();
        return Ok(());
    }

    let db_path = parse_arg(&args, "--db").ok_or("Missing --db")?;
    let collection_name = parse_arg(&args, "--collection").ok_or("Missing --collection")?;
    let document_path = parse_arg(&args, "--document").ok_or("Missing --document")?;
    let output_path = parse_arg(&args, "--output").ok_or("Missing --output")?;
    let schema_path = parse_arg(&args, "--schema");

    let db = DatabaseCore::<StorageEngine>::open(db_path.as_str())?;
    if let Some(schema_file) = schema_path {
        let schema_json: Value = serde_json::from_str(&fs::read_to_string(schema_file)?)?;
        db.set_collection_schema(&collection_name, Some(schema_json))?;
    }

    let collection = db.collection(&collection_name)?;

    let raw_doc: Value = serde_json::from_str(&fs::read_to_string(&document_path)?)?;
    let fields = value_to_map(raw_doc)?;
    let inserted = collection.insert_one(fields)?;
    let doc_id_value = serde_json::to_value(&inserted)?;

    let stored = collection
        .find_one(&json!({ "_id": doc_id_value }))
        .map_err(|e| format!("find_one failed: {e}"))?
        .ok_or("Document not found after insert")?;

    let mut cleaned = stored.clone();
    remove_level(&mut cleaned);

    let mut output = cleaned;
    if let Some(map) = output.as_object_mut() {
        map.remove("_id");
        map.remove("_collection");
    }

    let output_path = PathBuf::from(output_path);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, serde_json::to_string_pretty(&output)?)?;

    println!("Cleaned document written successfully");
    Ok(())
}
