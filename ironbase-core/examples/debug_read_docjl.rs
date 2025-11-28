// Debug script to read docjl_storage.mlite using ironbase-core
// This mimics what the MCP server does

use ironbase_core::{storage::StorageEngine, DatabaseCore};
use std::path::PathBuf;

fn main() {
    let separator = "=".repeat(80);
    println!("{}", separator);
    println!("DEBUG: Reading docjl_storage.mlite with ironbase-core");
    println!("{}", separator);

    let db_path = PathBuf::from("../mcp-server/docjl_storage.mlite");

    println!("\n1. Opening database...");
    let db = match DatabaseCore::<StorageEngine>::open(&db_path) {
        Ok(db) => {
            println!("   ✅ Database opened successfully");
            db
        }
        Err(e) => {
            println!("   ❌ Failed to open database: {}", e);
            return;
        }
    };

    println!("\n2. Listing collections...");
    let collections = db.list_collections();
    println!(
        "   ✅ Found {} collection(s): {:?}",
        collections.len(),
        collections
    );

    println!("\n3. Getting 'documents' collection...");
    let collection = match db.collection("documents") {
        Ok(coll) => {
            println!("   ✅ Got collection");
            coll
        }
        Err(e) => {
            println!("   ❌ Failed to get collection: {}", e);
            return;
        }
    };

    println!("\n4. Counting documents...");
    match collection.count_documents(&serde_json::json!({})) {
        Ok(count) => {
            println!("   ✅ Count: {}", count);
        }
        Err(e) => {
            println!("   ❌ Failed to count: {}", e);
        }
    }

    println!("\n5. Finding all documents with find{{}}...");
    match collection.find(&serde_json::json!({})) {
        Ok(docs) => {
            println!("   ✅ Found {} document(s)", docs.len());
            for (i, doc) in docs.iter().enumerate() {
                println!("\n   Document {}:", i + 1);
                if let Some(id) = doc.get("_id") {
                    println!("   _id: {:?}", id);
                }
                if let Some(id) = doc.get("id") {
                    println!("   id: {:?}", id);
                }
                if let Some(blocks) = doc.get("blocks") {
                    if let Some(arr) = blocks.as_array() {
                        println!("   blocks: {} items", arr.len());
                    }
                }
                if let Some(docjll) = doc.get("docjll") {
                    if let Some(arr) = docjll.as_array() {
                        println!("   docjll: {} items", arr.len());
                    }
                }
            }
        }
        Err(e) => {
            println!("   ❌ Failed to find: {}", e);
        }
    }

    println!("\n{}", separator);
    println!("DEBUG COMPLETE");
    println!("{}", separator);
}
