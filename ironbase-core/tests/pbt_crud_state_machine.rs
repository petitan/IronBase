//! P2 (property-based testing plan): a random sequence of CRUD operations
//! keeps the engine equal to a plain `BTreeMap` model.
//!
//! The sequence mixes inserts, updates and deletes by `_id` and by filter,
//! index creation and removal, and on file storage checkpoints, compactions
//! and reopens. After every operation:
//! - `find({})` returns exactly the model's documents (same values);
//! - `count_documents({})` and the live document count equal the model size;
//! - `document_order` holds exactly the catalog's ids, each once;
//! - the matched counts of the writes equal what the oracle predicts;
//! - the probe queries (answered through whatever indexes exist) agree with
//!   the oracle on the model.

mod pbt_support;

use ironbase_core::storage::MemoryStorage;
use ironbase_core::{DatabaseCore, StorageEngine};
use pbt_support::crud::*;
use pbt_support::*;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(64)))]

    #[test]
    fn crud_sequence_keeps_memory_storage_equal_to_the_model(
        ops in prop::collection::vec(arb_crud_op(), 1..30),
        probes in prop::collection::vec(arb_query(), 1..3),
    ) {
        let db = DatabaseCore::<MemoryStorage>::open_memory().unwrap();
        db.insert_docs("s", &[]);
        let mut model = Model::new();
        let mut indexes = Vec::new();
        for op in &ops {
            apply(&db, &mut model, &mut indexes, op)?;
            check(&db, &model, &probes, op)?;
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(16)))]

    #[test]
    fn crud_sequence_keeps_file_storage_equal_to_the_model(
        ops in prop::collection::vec(arb_file_op(), 1..25),
        probes in prop::collection::vec(arb_query(), 1..3),
    ) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.mlite");
        let mut db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
        db.insert_docs("s", &[]);
        let mut model = Model::new();
        let mut indexes = Vec::new();
        for op in &ops {
            match op {
                Op::Checkpoint => {
                    db.checkpoint().unwrap();
                }
                Op::Compact => {
                    db.compact().unwrap();
                }
                Op::Reopen => {
                    db.close().unwrap();
                    drop(db);
                    db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
                }
                op => apply(&db, &mut model, &mut indexes, op)?,
            }
            check(&db, &model, &probes, op)?;
        }
        db.close().unwrap();
    }
}
