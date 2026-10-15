// vector_cache_path_tests.rs
// The HNSW cache file stays in the database directory whatever the
// collection name (MCP audit 2026-10-14 M6: `../x` wrote it outside).

use ironbase_core::vector::VectorIndexConfig;
use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;

#[test]
fn hnsw_cache_file_stays_in_database_directory() {
    let root = tempfile::TempDir::new().unwrap();
    let db_dir = root.path().join("db");
    std::fs::create_dir(&db_dir).unwrap();
    std::fs::create_dir(root.path().join("out")).unwrap();
    let db = DatabaseCore::open(db_dir.join("test.mlite")).unwrap();
    let name = "../out/pwn";
    let doc: HashMap<String, Value> = serde_json::from_value(json!({"v": [1.0, 2.0]})).unwrap();
    db.insert_one(name, doc).unwrap();
    db.collection(name)
        .unwrap()
        .create_vector_index("v", VectorIndexConfig::new(2))
        .unwrap();

    let escaped: Vec<_> = std::fs::read_dir(root.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        escaped.is_empty(),
        "cache file outside the db dir: {escaped:?}"
    );
    let in_db_dir = std::fs::read_dir(&db_dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".hnsw")
        })
        .count();
    assert_eq!(in_db_dir, 1);
}
