//! P10 (property-based testing plan): a restored backup is the database as
//! it was when the backup was taken.
//!
//! Random CRUD sequences on file storage with checkpoints, compactions,
//! clean reopens and backups mixed in (full or incremental, single file or
//! split into parts, taken while the database is open). The model records
//! the documents at every backup. At the end every backup of the current
//! chain is restored into an empty directory, and the latest one also over
//! the database it came from (its index files still next to it); each
//! restored database must hold exactly the recorded documents and answer
//! the B+ tree, fulltext and vector indexes the same way.
//!
//! An incremental backup asked for after a compaction must be refused
//! (`LayoutChanged`); every other backup must succeed.
//!
//! The database files may also be copied aside (closed) and one copy put
//! back by hand at the end, next to the index caches the database wrote
//! later: it must answer like a restored backup (the caches of a later
//! state are not loaded).

use ironbase_backup::{create_backup, restore, BackupError};
use ironbase_core::vector::VectorIndexConfig;
use ironbase_core::{DatabaseCore, FuzzyAlgorithm, StorageEngine};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

const WORDS: [&str; 5] = ["apple", "river", "stone", "cloud", "ember"];

/// A document of the model: group (B+ tree index) and word (fulltext and
/// fuzzy index); the vector comes from the id
type Model = BTreeMap<i64, (i64, usize)>;

#[derive(Debug, Clone)]
enum Op {
    Insert(i64, usize),
    /// Update the document the index picks (modulo the model's size)
    Update(usize, i64, usize),
    Delete(usize),
    Checkpoint,
    Compact,
    Reopen,
    /// Close the database and copy its files aside
    Copy,
    Backup {
        full: bool,
        split: bool,
    },
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        8 => (0i64..4, 0usize..WORDS.len()).prop_map(|(g, w)| Op::Insert(g, w)),
        3 => (any::<usize>(), 0i64..4, 0usize..WORDS.len()).prop_map(|(i, g, w)| Op::Update(i, g, w)),
        2 => any::<usize>().prop_map(Op::Delete),
        2 => Just(Op::Checkpoint),
        1 => Just(Op::Compact),
        1 => Just(Op::Reopen),
        1 => Just(Op::Copy),
        3 => (prop::bool::weighted(0.25), any::<bool>()).prop_map(|(full, split)| Op::Backup { full, split }),
    ]
}

fn vector_of(id: i64) -> [f32; 2] {
    let angle = id as f32 * 0.7;
    [angle.cos(), angle.sin()]
}

fn document(id: i64, g: i64, w: usize) -> HashMap<String, Value> {
    HashMap::from([
        ("_id".to_string(), json!(id)),
        ("g".to_string(), json!(g)),
        ("t".to_string(), json!(format!("{} {}", WORDS[w], id))),
        ("w".to_string(), json!(WORDS[w])),
        ("v".to_string(), json!(vector_of(id))),
    ])
}

fn open(path: &Path) -> DatabaseCore<StorageEngine> {
    let db = DatabaseCore::<StorageEngine>::open(path).unwrap();
    let c = db.collection("c").unwrap();
    // Idempotent: an index that exists already is kept
    let _ = c.create_index("g".to_string(), false, false);
    let _ = c.create_fulltext_index("t".to_string(), "english", None, None);
    let _ = c.create_vector_index("v", VectorIndexConfig::new(2));
    let _ = c.create_fuzzy_index("w".to_string(), FuzzyAlgorithm::JaroWinkler, 0.8);
    db
}

fn ids(docs: impl IntoIterator<Item = Value>) -> BTreeSet<i64> {
    docs.into_iter()
        .map(|d| d["_id"].as_i64().expect("integer _id"))
        .collect()
}

/// The database at `path` holds exactly the model's documents, and its
/// indexes agree with them
fn check(path: &Path, model: &Model, why: &str) -> Result<(), TestCaseError> {
    let db = DatabaseCore::<StorageEngine>::open(path)
        .map_err(|e| TestCaseError::fail(format!("{why}: open failed: {e}")))?;
    let docs = db.find("c", &json!({})).unwrap_or_default();
    let mut got: Model = Model::new();
    for d in &docs {
        let id = d["_id"].as_i64().unwrap();
        let g = d["g"].as_i64().unwrap();
        let word = d["t"].as_str().unwrap().split(' ').next().unwrap();
        let w = WORDS.iter().position(|x| *x == word).unwrap();
        prop_assert_eq!(
            d["v"].clone(),
            json!(vector_of(id)),
            "{}: vector of {}",
            why,
            id
        );
        got.insert(id, (g, w));
    }
    prop_assert_eq!(&got, model, "{}: documents", why);
    prop_assert_eq!(docs.len(), model.len(), "{}: duplicate documents", why);
    if model.is_empty() {
        return Ok(());
    }

    let c = db.collection("c").unwrap();
    for g in 0..4 {
        let want: BTreeSet<i64> = model
            .iter()
            .filter(|(_, v)| v.0 == g)
            .map(|(id, _)| *id)
            .collect();
        let found = ids(db.find("c", &json!({ "g": g })).unwrap());
        prop_assert_eq!(&found, &want, "{}: B+ tree index, g = {}", why, g);
    }
    for (w, word) in WORDS.iter().enumerate() {
        let want: BTreeSet<i64> = model
            .iter()
            .filter(|(_, v)| v.1 == w)
            .map(|(id, _)| *id)
            .collect();
        let found = ids(c
            .fulltext_search("t", word, None, None, None, None)
            .unwrap()
            .into_iter()
            .map(|(d, _, _)| d));
        prop_assert_eq!(&found, &want, "{}: fulltext index, {}", why, word);
        let found = ids(c
            .fuzzy_search("w", word, Some(1.0), None)
            .unwrap()
            .into_iter()
            .map(|(d, _)| d));
        prop_assert_eq!(&found, &want, "{}: fuzzy index, {}", why, word);
    }
    let live: BTreeSet<i64> = model.keys().cloned().collect();
    let found = ids(c
        .vector_search("v", &[1.0, 0.0], model.len())
        .unwrap()
        .into_iter()
        .map(|(d, _)| d));
    prop_assert!(
        found.is_subset(&live),
        "{}: vector index returns {:?}, live {:?}",
        why,
        found,
        live
    );
    prop_assert!(
        found.len() * 10 >= live.len() * 9,
        "{}: vector index returns {} of {}",
        why,
        found.len(),
        live.len()
    );
    drop(c);
    db.close().unwrap();
    Ok(())
}

/// Copy a closed database (its `.mlite` and `.wal`; not the index caches)
fn copy_database(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap();
    let (from_wal, to_wal) = (from.with_extension("wal"), to.with_extension("wal"));
    if from_wal.exists() {
        std::fs::copy(&from_wal, &to_wal).unwrap();
    } else if to_wal.exists() {
        std::fs::remove_file(&to_wal).unwrap();
    }
}

fn restore_to(backups: &Path, target: &str, out: &Path) -> Result<(), TestCaseError> {
    restore(backups, out, Some(target), Some("s"))
        .map(|_| ())
        .map_err(|e| TestCaseError::fail(format!("restore of {target} failed: {e}")))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(24)))]

    #[test]
    fn a_restored_backup_is_the_database_when_it_was_taken(
        ops in prop::collection::vec(arb_op(), 1..40),
        put_back in any::<usize>(),
    ) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("live").join("s.mlite");
        std::fs::create_dir(db_path.parent().unwrap()).unwrap();
        let backups = dir.path().join("backups");

        let mut db = open(&db_path);
        let mut model = Model::new();
        let mut next_id = 0i64;
        // Backups of the current chain: file name and the model then
        let mut chain: Vec<(String, Model)> = Vec::new();
        let mut compacted_since_backup = false;
        // Copies of the closed database files: directory and the model then
        let mut copies: Vec<(PathBuf, Model)> = Vec::new();

        for op in &ops {
            match op {
                Op::Insert(g, w) => {
                    db.insert_one("c", document(next_id, *g, *w)).unwrap();
                    model.insert(next_id, (*g, *w));
                    next_id += 1;
                }
                Op::Update(i, g, w) => {
                    if let Some(id) = model.keys().nth(i % model.len().max(1)).cloned() {
                        let t = format!("{} {}", WORDS[*w], id);
                        let set = json!({ "$set": { "g": g, "t": t, "w": WORDS[*w] } });
                        db.update_one("c", &json!({ "_id": id }), &set).unwrap();
                        model.insert(id, (*g, *w));
                    }
                }
                Op::Delete(i) => {
                    if let Some(id) = model.keys().nth(i % model.len().max(1)).cloned() {
                        db.delete_one("c", &json!({ "_id": id })).unwrap();
                        model.remove(&id);
                    }
                }
                Op::Checkpoint => {
                    db.checkpoint().unwrap();
                }
                Op::Compact => {
                    db.compact().unwrap();
                    compacted_since_backup = true;
                }
                Op::Reopen => {
                    db.close().unwrap();
                    drop(db);
                    db = open(&db_path);
                }
                Op::Copy => {
                    db.close().unwrap();
                    drop(db);
                    let copy = dir.path().join(format!("copy{}", copies.len()));
                    std::fs::create_dir(&copy).unwrap();
                    copy_database(&db_path, &copy.join("s.mlite"));
                    copies.push((copy, model.clone()));
                    db = open(&db_path);
                }
                Op::Backup { full, split } => {
                    let split = split.then_some(1024);
                    let result = create_backup(&db_path, &backups, *full, split);
                    let incremental = !*full && !chain.is_empty();
                    match result {
                        Err(BackupError::LayoutChanged) if incremental && compacted_since_backup => {}
                        Err(e) => prop_assert!(false, "backup ({:?}) failed: {}", op, e),
                        Ok(r) => {
                            prop_assert!(
                                !(incremental && compacted_since_backup),
                                "an incremental backup after a compaction was taken"
                            );
                            if !incremental {
                                chain.clear();
                            }
                            let name = r.path.file_name().unwrap().to_string_lossy().into_owned();
                            chain.push((name, model.clone()));
                            compacted_since_backup = false;
                        }
                    }
                }
            }
        }

        // Each backup of the chain, into an empty directory
        for (k, (name, snapshot)) in chain.iter().enumerate() {
            let out_dir = dir.path().join(format!("restored{k}"));
            std::fs::create_dir(&out_dir).unwrap();
            let out = out_dir.join("s.mlite");
            restore_to(&backups, name, &out)?;
            check(&out, snapshot, &format!("restore of backup {k} ({name})"))?;
        }

        // A copy put back by hand over the database, its caches left as the
        // database wrote them later
        db.close().unwrap();
        drop(db);
        if !copies.is_empty() {
            let (copy, snapshot) = &copies[put_back % copies.len()];
            copy_database(&copy.join("s.mlite"), &db_path);
            check(&db_path, snapshot, &format!("copy {} put back", copy.display()))?;
        }

        // The latest backup restored over the database it came from
        if let Some((name, snapshot)) = chain.last() {
            restore_to(&backups, name, &db_path)?;
            check(&db_path, snapshot, &format!("in-place restore of {name}"))?;
        }
    }
}
