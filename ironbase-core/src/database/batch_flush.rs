//! Batch-mode flush, shared by `DatabaseCore` and every `CollectionCore`
//! handle of the database.
//!
//! `DatabaseCore::flush_batch` flushes every buffered insert (batch full,
//! update/delete, checkpoint, transaction start). A handle's read entry points
//! flush only their own collection through `PendingWrites::flush_for_read`, so
//! a read sees every insert acknowledged before it.

use super::{begin_auto_tx, try_enter_auto_write_now, BatchDocBuffer, WriteLockState};
use crate::collection_core::{CollectionCore, InsertOnePrepared, PendingWrites, RawOperations};
use crate::document::DocumentId;
use crate::error::Result;
use crate::storage::StorageEngine;
use crate::transaction::Operation;
use parking_lot::{Condvar, Mutex, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The state a batch flush needs, shared (`Arc`) with `DatabaseCore`.
pub(crate) struct BatchFlusher {
    pub(crate) storage: Arc<RwLock<StorageEngine>>,
    pub(crate) batch_buffer: Arc<RwLock<Vec<Operation>>>,
    pub(crate) doc_buffer: Arc<RwLock<BatchDocBuffer>>,
    pub(crate) persist_gate: Arc<RwLock<()>>,
    pub(crate) next_tx_id: Arc<AtomicU64>,
    pub(crate) in_flight_tx_ids: Arc<Mutex<std::collections::BTreeSet<u64>>>,
    pub(crate) max_committed_tx_id: Arc<AtomicU64>,
    pub(crate) write_transaction_lock: Arc<Mutex<WriteLockState>>,
    pub(crate) write_lock_condvar: Arc<Condvar>,
}

impl BatchFlusher {
    /// Flush buffered inserts with WAL-first ordering: one auto-transaction
    /// holding their WAL operations is committed (WAL fsync = atomic point),
    /// then the documents are persisted through `persist`, then the file is
    /// synced. `only` restricts the flush to one collection; the others stay
    /// buffered.
    ///
    /// All or nothing: if a document fails to persist, the documents this
    /// flush already persisted are removed again (`rollback`) and the
    /// transaction is aborted in the WAL, so storage, indexes and WAL
    /// recovery agree that none of it happened.
    pub(crate) fn flush(
        &self,
        only: Option<&str>,
        persist: &mut dyn FnMut(&str, InsertOnePrepared) -> Result<()>,
        rollback: &mut dyn FnMut(&str, &DocumentId) -> Result<()>,
    ) -> Result<()> {
        let mut batch = self.batch_buffer.write();
        let mut doc_buffer = self.doc_buffer.write();
        let selected = |op: &Operation| only.is_none_or(|name| op_collection(op) == name);
        let nothing_buffered = match only {
            None => doc_buffer.is_empty(),
            Some(name) => !doc_buffer.inserts.contains_key(name),
        };
        if nothing_buffered && !batch.iter().any(selected) {
            return Ok(());
        }

        // 1. WAL COMMIT FIRST (atomic point). The buffers are only emptied
        // after it succeeds, so a failed commit loses nothing. Hold the
        // persist gate until the storage write is done, so a checkpoint
        // cannot clear this commit from the WAL before then.
        let _persist_gate = self.persist_gate.read();
        let (mut auto_tx, _in_flight) = begin_auto_tx(&self.next_tx_id, &self.in_flight_tx_ids);
        let tx_id = auto_tx.id;
        for op in batch.iter().filter(|op| selected(op)) {
            auto_tx.add_operation(op.clone())?;
        }
        // Persisted below, not by commit_transaction's apply_operations()
        auto_tx.mark_operations_applied();
        {
            let mut storage = self.storage.write();
            storage.commit_transaction_batch(&mut auto_tx)?;
        }
        self.max_committed_tx_id.fetch_max(tx_id, Ordering::SeqCst);

        // 2. Take the committed operations and documents out of the buffers
        batch.retain(|op| !selected(op));
        let docs: Vec<(String, Vec<InsertOnePrepared>)> = match only {
            None => {
                let docs = doc_buffer.inserts.drain().collect();
                let previous_bytes = doc_buffer.memory_bytes;
                doc_buffer.clear_and_shrink_if_large(previous_bytes);
                docs
            }
            Some(name) => vec![(name.to_string(), doc_buffer.take_collection(name))],
        };

        // 3. PERSIST the buffered documents. A crash from here on is replayed
        // from the WAL.
        let mut persisted: Vec<(String, DocumentId)> = Vec::new();
        for (name, prepared) in docs {
            for p in prepared {
                let doc_id = p.doc_id.clone();
                if let Err(e) = persist(&name, p) {
                    // Undo this flush's persisted documents, then ABORT so
                    // recovery skips the transaction too
                    for (coll, id) in persisted.iter().rev() {
                        if let Err(undo_err) = rollback(coll, id) {
                            tracing::warn!(
                                tx_id = tx_id,
                                collection = %coll,
                                doc_id = ?id,
                                error = %undo_err,
                                "Failed to roll back a persisted document of a failed batch flush"
                            );
                        }
                    }
                    if let Err(abort_err) = self.storage.write().write_abort_entry(tx_id) {
                        tracing::warn!(
                            tx_id = tx_id,
                            error = %abort_err,
                            "Failed to write WAL ABORT entry after persist failure — recovery may replay this transaction"
                        );
                    }
                    return Err(e);
                }
                persisted.push((name.clone(), doc_id));
            }
        }

        // 4. Sync storage file (one fsync per batch - key optimization)
        drop(batch);
        drop(doc_buffer);
        self.storage.write().sync_file()?;
        Ok(())
    }
}

/// Collection of a buffered WAL operation (only inserts are buffered).
fn op_collection(op: &Operation) -> &str {
    match op {
        Operation::Insert { collection, .. }
        | Operation::Update { collection, .. }
        | Operation::Delete { collection, .. } => collection,
    }
}

impl PendingWrites<StorageEngine> for BatchFlusher {
    fn has_pending(&self, collection: &str) -> bool {
        self.doc_buffer.read().inserts.contains_key(collection)
    }

    fn flush_all(
        &self,
        persist: &mut dyn FnMut(&str, InsertOnePrepared) -> Result<()>,
        rollback: &mut dyn FnMut(&str, &DocumentId) -> Result<()>,
    ) -> Result<()> {
        self.flush(None, persist, rollback)
    }

    fn flush_for_read(&self, collection: &CollectionCore<StorageEngine>) -> Result<()> {
        // A flush is a write: hold the write lock shared like every
        // auto-commit, so an explicit transaction never sees it half done
        let Some(_auto_write) =
            try_enter_auto_write_now(&self.write_transaction_lock, &self.write_lock_condvar)
        else {
            return Ok(());
        };
        self.flush(
            Some(&collection.name),
            &mut |_, prepared| collection.insert_one_persist(prepared).map(|_| ()),
            &mut |_, doc_id| collection.remove_document_by_id(doc_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::collection_core::RawOperations;
    use crate::durability::DurabilityMode;
    use crate::storage::StorageEngine;
    use crate::DatabaseCore;
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn f(d: Value) -> HashMap<String, Value> {
        d.as_object().unwrap().clone().into_iter().collect()
    }

    fn ids(db: &DatabaseCore<StorageEngine>) -> Vec<i64> {
        let mut v: Vec<i64> = db
            .find("c", &json!({}))
            .unwrap()
            .iter()
            .map(|d| d["_id"].as_i64().unwrap())
            .collect();
        v.sort();
        v
    }

    /// A persist failure in the middle of a flush leaves no half-applied
    /// batch: the documents already persisted by that flush are rolled back,
    /// so storage matches the WAL ABORT, before and after a reopen.
    #[test]
    fn persist_failure_rolls_back_the_whole_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.mlite");
        let open = || {
            DatabaseCore::<StorageEngine>::open_with_durability(
                &path,
                DurabilityMode::Batch { batch_size: 100 },
            )
            .unwrap()
        };
        {
            let db = open();
            db.insert_one("c", f(json!({"_id": 1}))).unwrap();
            db.insert_one("c", f(json!({"_id": 5}))).unwrap();
            // A write that bypasses the buffer takes _id 5 first, so the
            // flush fails on the second buffered document
            db.collection("c")
                .unwrap()
                .insert_one_raw(f(json!({"_id": 5, "raw": true})))
                .unwrap();
            assert!(db.flush_batch().is_err());
            assert_eq!(ids(&db), vec![5]);
            db.close().unwrap();
        }
        let db = open();
        assert_eq!(ids(&db), vec![5]);
        let five = db.find_one("c", &json!({"_id": 5})).unwrap().unwrap();
        assert_eq!(five["raw"], json!(true));
    }
}
