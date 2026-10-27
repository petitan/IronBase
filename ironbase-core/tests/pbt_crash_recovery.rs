//! P3 (property-based testing plan): crashes at random points of a CRUD
//! sequence lose nothing that was acknowledged and apply nothing halfway.
//!
//! The P2 state machine on file storage (Safe durability, the default), with
//! process crashes mixed in: the files are taken as the running database left
//! them on disk (no flush, no clean-shutdown mark) and opened again, which
//! takes the recovery path. A crash may also hit a transaction
//! in flight, modelled at the end of the WAL:
//! - cut after any number of bytes before its commit: it must leave no trace;
//! - committed to the WAL but not yet written to the data file: recovery must
//!   apply it entirely, indexes included.
//!
//! After every step the P2 invariants hold against the model.

mod pbt_support;

use ironbase_core::document::DocumentId;
use ironbase_core::transaction::Operation;

use ironbase_core::{DatabaseCore, StorageEngine, WALEntry, WALEntryType};
use pbt_support::crud::*;
use pbt_support::*;
use proptest::prelude::*;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone)]
enum Step {
    /// A P2 operation, checkpoint, compaction or clean reopen
    Op(Op),
    /// The process dies after the last acknowledged write
    Crash,
    /// ... while writing an insert's WAL entries: cut after this many bytes
    /// (modulo their length), before the commit
    CrashInUncommittedWal(usize, Map<String, Value>),
    /// ... after the insert's WAL commit, before its data-file write
    CrashAfterWalCommit(Map<String, Value>),
}

fn arb_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        12 => arb_file_op().prop_map(Step::Op),
        2 => Just(Step::Crash),
        1 => (any::<usize>(), arb_doc()).prop_map(|(cut, doc)| Step::CrashInUncommittedWal(cut, doc)),
        1 => arb_doc().prop_map(Step::CrashAfterWalCommit),
    ]
}

/// The database files, one directory per process lifetime
struct Files {
    root: tempfile::TempDir,
    generation: usize,
}

impl Files {
    fn new() -> Self {
        let files = Files {
            root: tempfile::TempDir::new().unwrap(),
            generation: 0,
        };
        std::fs::create_dir(files.dir()).unwrap();
        files
    }

    fn dir(&self) -> std::path::PathBuf {
        self.root.path().join(self.generation.to_string())
    }

    fn db(&self) -> std::path::PathBuf {
        self.dir().join("s.mlite")
    }

    fn wal(&self) -> std::path::PathBuf {
        self.db().with_extension("wal")
    }

    /// Kill the process holding `db`: what it wrote is on disk, nothing
    /// more. The files are copied as they are into the next generation's
    /// directory, the next open uses that one; `db` itself is then dropped
    /// normally (its shutdown writes into the abandoned directory). Unlike
    /// `mem::forget`, nothing leaks: thousands of cases would run out of
    /// file descriptors.
    fn crash(&mut self, db: DatabaseCore<StorageEngine>) {
        let from = self.dir();
        self.generation += 1;
        copy_dir(&from, &self.dir());
        drop(db);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else if entry.path().extension().is_some_and(|e| e == "lock") {
            // The live process's lock file: a killed process's lock is gone
            // with it, and the next open rewrites the file. Windows refuses to
            // read a file another handle holds a byte-range lock on.
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The WAL entries of a transaction inserting `doc` with `_id` `id`
fn insert_transaction(tx_id: u64, id: &str, doc: &Map<String, Value>, commit: bool) -> Vec<u8> {
    let mut stored = doc.clone();
    stored.insert("_id".into(), json!(id));
    stored.insert("_collection".into(), json!("s"));
    let op = Operation::Insert {
        collection: "s".to_string(),
        doc_id: DocumentId::String(id.to_string()),
        doc: std::sync::Arc::new(Value::Object(stored)),
    };
    let mut bytes = WALEntry::new(tx_id, WALEntryType::Begin, vec![]).serialize();
    bytes.extend(
        WALEntry::new(
            tx_id,
            WALEntryType::Operation,
            serde_json::to_vec(&op).unwrap(),
        )
        .serialize(),
    );
    if commit {
        bytes.extend(WALEntry::new(tx_id, WALEntryType::Commit, vec![]).serialize());
    }
    bytes
}

fn append(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(24)))]

    #[test]
    fn crashes_lose_no_acknowledged_write_and_apply_none_halfway(
        steps in prop::collection::vec(arb_step(), 1..25),
        probes in prop::collection::vec(arb_query(), 1..3),
    ) {
        let mut files = Files::new();
        let mut db = DatabaseCore::<StorageEngine>::open(files.db()).unwrap();
        db.insert_docs("s", &[]);
        let mut model = Model::new();
        let mut indexes = Vec::new();
        for (n, step) in steps.iter().enumerate() {
            // In-flight transactions: ids no auto-assigned id takes; tx ids
            // above any used before (the engine continues after the highest
            // recovered one, so each in-flight one leaves 2^40 to the next)
            let id = format!("in-flight-{n}");
            let tx_id = (n as u64 + 1) << 40;
            match step {
                Step::Op(Op::Checkpoint) => {
                    db.checkpoint().unwrap();
                }
                Step::Op(Op::Compact) => {
                    db.compact().unwrap();
                }
                Step::Op(Op::Reopen) => {
                    db.close().unwrap();
                    drop(db);
                    db = DatabaseCore::<StorageEngine>::open(files.db()).unwrap();
                }
                Step::Op(op) => apply(&db, &mut model, &mut indexes, op)?,
                Step::Crash => {
                    files.crash(db);
                    db = DatabaseCore::<StorageEngine>::open(files.db()).unwrap();
                }
                Step::CrashInUncommittedWal(cut, doc) => {
                    files.crash(db);
                    let bytes = insert_transaction(tx_id, &id, doc, false);
                    append(&files.wal(), &bytes[..1 + cut % bytes.len()]);
                    db = DatabaseCore::<StorageEngine>::open(files.db()).unwrap();
                }
                Step::CrashAfterWalCommit(doc) => {
                    files.crash(db);
                    append(&files.wal(), &insert_transaction(tx_id, &id, doc, true));
                    db = DatabaseCore::<StorageEngine>::open(files.db()).unwrap();
                    let mut stored = doc.clone();
                    stored.insert("_id".into(), json!(id));
                    model.insert(json!(id).to_string(), stored);
                }
            }
            let shown = match step {
                Step::Op(op) => op.clone(),
                _ => Op::Reopen,
            };
            check(&db, &model, &probes, &shown).map_err(|e| {
                TestCaseError::fail(format!("after step {n} {step:?}: {e}"))
            })?;
        }
        db.close().unwrap();
    }
}
