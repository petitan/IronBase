#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::Arbitrary;
use ironbase_core::DatabaseCore;
use ironbase_core::storage::MemoryStorage;
use std::collections::HashMap;
use serde_json::json;

// Structured fuzzing input for fulltext search
#[derive(Debug, Arbitrary)]
struct FuzzInput {
    // Text content to index
    text1: String,
    text2: String,
    text3: String,
    // Search query
    query: String,
    // Language selection (0-3)
    language: u8,
    // Options
    min_word_length: u8,
    limit: u8,
}

// Fuzz target: Fulltext search indexing and querying
// Goal: Find panics in tokenization, stemming, or TF-IDF scoring

fuzz_target!(|input: FuzzInput| {
    // Skip empty queries
    if input.query.is_empty() {
        return;
    }

    let db = match DatabaseCore::<MemoryStorage>::open_memory() {
        Ok(db) => db,
        Err(_) => return,
    };

    // Select language based on fuzz input
    let language = match input.language % 4 {
        0 => "hungarian",
        1 => "english",
        2 => "german",
        _ => "none",
    };

    // Get or create collection
    let collection = match db.collection("fuzz_fts") {
        Ok(c) => c,
        Err(_) => return,
    };

    // Create fulltext index - should not panic
    let min_word_len = if input.min_word_length == 0 { None } else { Some(input.min_word_length as usize) };
    let _ = collection.create_fulltext_index(
        "content".to_string(),
        language,
        min_word_len,
        Some(true), // accent folding
    );

    // Insert documents with fuzzed text
    for (i, text) in [&input.text1, &input.text2, &input.text3].iter().enumerate() {
        let doc = HashMap::from([
            ("_id".to_string(), json!(i)),
            ("content".to_string(), json!(text)),
            ("title".to_string(), json!(format!("Doc {}", i))),
        ]);
        let _ = db.insert_one("fuzz_fts", doc);
    }

    // Execute fulltext search - should NEVER panic
    let limit = if input.limit == 0 { 10 } else { input.limit as usize };
    let _ = collection.fulltext_search(
        "content",
        &input.query,
        Some(limit),
        None, // skip
        None, // min_score
        None, // projection
    );

    // Test with special characters in query
    let special_query = format!("{}$%^&*()", input.query);
    let _ = collection.fulltext_search("content", &special_query, Some(10), None, None, None);

    // Test with very long query
    let long_query = input.query.repeat(100);
    if long_query.len() < 10000 { // Avoid OOM
        let _ = collection.fulltext_search("content", &long_query, Some(10), None, None, None);
    }

    // Test update and delete with fulltext index
    let _ = db.update_one(
        "fuzz_fts",
        &json!({"_id": 0}),
        &json!({"$set": {"content": input.query}})
    );
    let _ = db.delete_one("fuzz_fts", &json!({"_id": 1}));

    // Final search after modifications
    let _ = collection.fulltext_search("content", &input.query, Some(10), None, None, None);
});
