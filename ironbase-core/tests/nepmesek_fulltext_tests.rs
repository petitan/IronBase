//! Integration tests for Hungarian fulltext search on folk tales (népmesék)
//!
//! Tests the fulltext search functionality with real Hungarian text content.
//! Uses nested document structure with paragraphs.

use ironbase_core::DatabaseCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper macro to create a document HashMap
macro_rules! doc {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map: HashMap<String, Value> = HashMap::new();
        $(
            map.insert($key.to_string(), json!($value));
        )*
        map
    }};
}

/// Create test database with sample Hungarian folk tales
fn create_nepmesek_db() -> (TempDir, std::path::PathBuf) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nepmesek_test.mlite");

    let db = DatabaseCore::open(&db_path).unwrap();

    // A vak király - részlet
    db.insert_one("mesek", doc!{
        "title" => "A vak király",
        "index" => 1,
        "metadata" => json!({
            "word_count": 100,
            "source": "MEK"
        }),
        "paragraphs" => json!([
            {"num": 1, "text": "Hol volt, hol nem volt, volt egyszer egy király."},
            {"num": 2, "text": "Ez a király olyan vak volt, hogy semmit sem látott."},
            {"num": 3, "text": "A királykisasszony nagyon szép volt és okos."}
        ]),
        "full_text" => "Hol volt, hol nem volt, volt egyszer egy király. Ez a király olyan vak volt, hogy semmit sem látott. A királykisasszony nagyon szép volt és okos."
    }).unwrap();

    // Fehérlófia
    db.insert_one("mesek", doc!{
        "title" => "Fehérlófia",
        "index" => 12,
        "metadata" => json!({
            "word_count": 150,
            "source": "MEK"
        }),
        "paragraphs" => json!([
            {"num": 1, "text": "Egyszer volt, hol nem volt, volt egyszer egy fehér ló."},
            {"num": 2, "text": "Ennek a lónak született egy fia, akit Fehérlófiának hívtak."},
            {"num": 3, "text": "Fehérlófia olyan erős volt, hogy a sárkányt is legyőzte."},
            {"num": 4, "text": "A sárkány tüzet okádott, de Fehérlófia nem félt."}
        ]),
        "full_text" => "Egyszer volt, hol nem volt, volt egyszer egy fehér ló. Ennek a lónak született egy fia, akit Fehérlófiának hívtak. Fehérlófia olyan erős volt, hogy a sárkányt is legyőzte. A sárkány tüzet okádott, de Fehérlófia nem félt."
    }).unwrap();

    // Babszem Jankó
    db.insert_one("mesek", doc!{
        "title" => "Babszem Jankó",
        "index" => 19,
        "metadata" => json!({
            "word_count": 120,
            "source": "MEK"
        }),
        "paragraphs" => json!([
            {"num": 1, "text": "Volt egyszer egy öregasszony, akinek nem volt gyereke."},
            {"num": 2, "text": "Egyszer talált egy babszemet és abból született Babszem Jankó."},
            {"num": 3, "text": "Jankó kicsi volt, mint egy babszem, de nagyon okos."},
            {"num": 4, "text": "A király meghallotta hírét és magához hívatta."}
        ]),
        "full_text" => "Volt egyszer egy öregasszony, akinek nem volt gyereke. Egyszer talált egy babszemet és abból született Babszem Jankó. Jankó kicsi volt, mint egy babszem, de nagyon okos. A király meghallotta hírét és magához hívatta."
    }).unwrap();

    // Zsuzska és az ördög
    db.insert_one("mesek", doc!{
        "title" => "Zsuzska és az ördög",
        "index" => 22,
        "metadata" => json!({
            "word_count": 130,
            "source": "MEK"
        }),
        "paragraphs" => json!([
            {"num": 1, "text": "Élt egyszer egy szegény lány, akit Zsuzskának hívtak."},
            {"num": 2, "text": "Az ördög el akarta vinni Zsuzskát a pokolba."},
            {"num": 3, "text": "De Zsuzska okosabb volt az ördögnél."},
            {"num": 4, "text": "Végül az ördög kudarcot vallott és elmenekült."}
        ]),
        "full_text" => "Élt egyszer egy szegény lány, akit Zsuzskának hívtak. Az ördög el akarta vinni Zsuzskát a pokolba. De Zsuzska okosabb volt az ördögnél. Végül az ördög kudarcot vallott és elmenekült."
    }).unwrap();

    // A kis malac és a farkasok
    db.insert_one("mesek", doc!{
        "title" => "A kis malac és a farkasok",
        "index" => 17,
        "metadata" => json!({
            "word_count": 90,
            "source": "MEK"
        }),
        "paragraphs" => json!([
            {"num": 1, "text": "Volt egyszer három kis malac."},
            {"num": 2, "text": "A farkasok meg akarták enni őket."},
            {"num": 3, "text": "De a malacok okosak voltak és házat építettek."},
            {"num": 4, "text": "A farkas fújt és fújt, de a téglaházat nem tudta ledönteni."}
        ]),
        "full_text" => "Volt egyszer három kis malac. A farkasok meg akarták enni őket. De a malacok okosak voltak és házat építettek. A farkas fújt és fújt, de a téglaházat nem tudta ledönteni."
    }).unwrap();

    drop(db);
    (temp_dir, db_path)
}

// ============================================================================
// HUNGARIAN FULLTEXT SEARCH TESTS
// ============================================================================

#[test]
fn test_nepmesek_fulltext_basic_search() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    // Create fulltext index with Hungarian language
    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for "király" (king)
    let results = coll
        .fulltext_search("full_text", "király", Some(10), None, None, None)
        .unwrap();

    assert!(results.len() >= 2, "Should find documents with 'király'");

    // Verify titles
    let titles: Vec<String> = results
        .iter()
        .filter_map(|(doc, _, _)| doc.get("title").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .collect();

    assert!(titles.contains(&"A vak király".to_string()));
    assert!(titles.contains(&"Babszem Jankó".to_string()));
}

#[test]
fn test_nepmesek_fulltext_hungarian_stop_words() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // "volt" is a Hungarian stop word (was/were)
    let results = coll
        .fulltext_search("full_text", "volt", Some(10), None, None, None)
        .unwrap();

    assert_eq!(
        results.len(),
        0,
        "'volt' should be filtered as Hungarian stop word"
    );

    // "egy" is also a stop word (a/an/one)
    let results = coll
        .fulltext_search("full_text", "egy", Some(10), None, None, None)
        .unwrap();

    assert_eq!(
        results.len(),
        0,
        "'egy' should be filtered as Hungarian stop word"
    );

    // "az" is a stop word (the)
    let results = coll
        .fulltext_search("full_text", "az", Some(10), None, None, None)
        .unwrap();

    assert_eq!(
        results.len(),
        0,
        "'az' should be filtered as Hungarian stop word"
    );
}

#[test]
fn test_nepmesek_fulltext_accent_search() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    // Create index with accent folding
    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, Some(true))
        .unwrap();

    // Search without accent should find accented text
    let results = coll
        .fulltext_search("full_text", "ordog", Some(10), None, None, None)
        .unwrap();

    assert!(
        !results.is_empty(),
        "Should find 'ördög' when searching 'ordog' (accent folding)"
    );

    // Search for "sarkany" should find "sárkány"
    let results = coll
        .fulltext_search("full_text", "sarkany", Some(10), None, None, None)
        .unwrap();

    assert!(
        !results.is_empty(),
        "Should find 'sárkány' when searching 'sarkany'"
    );
}

#[test]
fn test_nepmesek_fulltext_character_names() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for character names
    let results = coll
        .fulltext_search("full_text", "Fehérlófia", Some(10), None, None, None)
        .unwrap();

    assert_eq!(
        results.len(),
        1,
        "Should find exactly 1 document with 'Fehérlófia'"
    );
    let doc = &results[0].0;
    assert_eq!(doc.get("title").unwrap(), "Fehérlófia");

    // Search for Zsuzska
    let results = coll
        .fulltext_search("full_text", "Zsuzska", Some(10), None, None, None)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0.get("title").unwrap(), "Zsuzska és az ördög");

    // Search for Jankó (appears in multiple tales)
    let results = coll
        .fulltext_search("full_text", "Jankó", Some(10), None, None, None)
        .unwrap();

    assert!(!results.is_empty(), "Should find documents with 'Jankó'");
}

#[test]
fn test_nepmesek_fulltext_creature_search() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for "sárkány" (dragon)
    let results = coll
        .fulltext_search("full_text", "sárkány", Some(10), None, None, None)
        .unwrap();

    assert!(!results.is_empty(), "Should find documents with 'sárkány'");
    assert!(results
        .iter()
        .any(|(doc, _, _)| doc.get("title").unwrap() == "Fehérlófia"));

    // Search for "farkas" (wolf)
    let results = coll
        .fulltext_search("full_text", "farkas", Some(10), None, None, None)
        .unwrap();

    assert!(!results.is_empty(), "Should find documents with 'farkas'");

    // Search for "malac" (pig)
    let results = coll
        .fulltext_search("full_text", "malac", Some(10), None, None, None)
        .unwrap();

    assert!(!results.is_empty(), "Should find documents with 'malac'");
}

#[test]
fn test_nepmesek_fulltext_tfidf_ranking() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for "okos" (smart/clever) - appears in multiple tales
    let results = coll
        .fulltext_search("full_text", "okos", Some(10), None, None, None)
        .unwrap();

    assert!(
        results.len() >= 2,
        "Should find multiple documents with 'okos'"
    );

    // Results should be sorted by TF-IDF score (descending)
    for i in 1..results.len() {
        assert!(
            results[i - 1].1 >= results[i].1,
            "Results should be sorted by score descending"
        );
    }
}

#[test]
fn test_nepmesek_fulltext_multi_word_query() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Multi-word query (each word scored separately)
    let results = coll
        .fulltext_search("full_text", "király szép", Some(10), None, None, None)
        .unwrap();

    assert!(
        !results.is_empty(),
        "Should find documents matching 'király szép'"
    );

    // "A vak király" contains both words
    assert!(results
        .iter()
        .any(|(doc, _, _)| doc.get("title").unwrap() == "A vak király"));
}

#[test]
fn test_nepmesek_fulltext_nested_metadata_preserved() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    let results = coll
        .fulltext_search("full_text", "Fehérlófia", Some(1), None, None, None)
        .unwrap();

    assert_eq!(results.len(), 1);

    let doc = &results[0].0;

    // Verify nested metadata is preserved
    let metadata = doc.get("metadata").unwrap();
    assert!(metadata.get("word_count").is_some());
    assert!(metadata.get("source").is_some());
    assert_eq!(metadata.get("source").unwrap(), "MEK");

    // Verify paragraphs array is preserved
    let paragraphs = doc.get("paragraphs").unwrap().as_array().unwrap();
    assert!(!paragraphs.is_empty());
    assert!(paragraphs[0].get("num").is_some());
    assert!(paragraphs[0].get("text").is_some());
}

#[test]
fn test_nepmesek_fulltext_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("nepmesek_persist.mlite");

    // Phase 1: Create database, add data, create fulltext index
    {
        let db = DatabaseCore::open(&db_path).unwrap();

        db.insert_one(
            "mesek",
            doc! {
                "title" => "Tündér Ilona",
                "full_text" => "Volt egyszer egy tündér, akit Ilonának hívtak. A tündér varázslatos volt."
            },
        )
        .unwrap();

        let coll = db.collection("mesek").unwrap();
        coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
            .unwrap();

        // Verify search works
        let results = coll
            .fulltext_search("full_text", "tündér", Some(10), None, None, None)
            .unwrap();
        assert_eq!(results.len(), 1);
    }
    // db dropped - should flush

    // Phase 2: Reopen and verify fulltext index persisted
    {
        let db2 = DatabaseCore::open(&db_path).unwrap();
        let coll2 = db2.collection("mesek").unwrap();

        let results = coll2
            .fulltext_search("full_text", "tündér", Some(10), None, None, None)
            .unwrap();

        assert!(
            !results.is_empty(),
            "Fulltext search should work after database reload"
        );
        assert_eq!(results[0].0.get("title").unwrap(), "Tündér Ilona");

        // Also test accent folding after reload
        let results = coll2
            .fulltext_search("full_text", "varazslatos", Some(10), None, None, None)
            .unwrap();
        assert!(
            !results.is_empty(),
            "Accent folding should work after reload"
        );
    }
}

#[test]
fn test_nepmesek_fulltext_no_results() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for word that doesn't exist
    let results = coll
        .fulltext_search("full_text", "számítógép", Some(10), None, None, None)
        .unwrap();

    assert_eq!(results.len(), 0, "Should find no results for 'számítógép'");

    // Search for English word
    let results = coll
        .fulltext_search("full_text", "computer", Some(10), None, None, None)
        .unwrap();

    assert_eq!(results.len(), 0, "Should find no results for English word");
}

#[test]
fn test_nepmesek_fulltext_pagination() {
    let (_temp_dir, db_path) = create_nepmesek_db();
    let db = DatabaseCore::open(&db_path).unwrap();
    let coll = db.collection("mesek").unwrap();

    coll.create_fulltext_index("full_text".to_string(), "hungarian", None, None)
        .unwrap();

    // Search for "király" which appears in multiple documents
    let all_results = coll
        .fulltext_search("full_text", "király", Some(10), None, None, None)
        .unwrap();

    assert!(
        all_results.len() >= 2,
        "Should have at least 2 results for pagination test"
    );

    // Get first result only
    let first = coll
        .fulltext_search("full_text", "király", Some(1), Some(0), None, None)
        .unwrap();
    assert_eq!(first.len(), 1, "Limit 1 should return 1 result");

    // Skip first, get second
    let second = coll
        .fulltext_search("full_text", "király", Some(1), Some(1), None, None)
        .unwrap();
    assert_eq!(second.len(), 1, "Skip 1, limit 1 should return 1 result");

    // They should be different documents
    assert_ne!(
        first[0].0.get("_id"),
        second[0].0.get("_id"),
        "Pagination should return different documents"
    );

    // Verify total count with higher limit
    let all = coll
        .fulltext_search("full_text", "király", Some(10), Some(0), None, None)
        .unwrap();
    assert_eq!(all.len(), all_results.len(), "All results should match");
}
