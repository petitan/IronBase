// src/database/transactions.rs
// Transaction lifecycle, write locks, and transaction API

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use serde_json::Value;

use crate::document::DocumentId;
use crate::error::{IronBaseError, Result};
use crate::storage::{RawStorage, Storage, StorageEngine};
use crate::transaction::{Transaction, TransactionId};

use super::DatabaseCore;

// ============================================================================
// STORAGEENGINE-SPECIFIC TRANSACTION METHODS
// ============================================================================

impl DatabaseCore<StorageEngine> {
    /// Commit a transaction (applies all buffered operations atomically) - StorageEngine-specific
    ///
    /// Automatically releases the write lock on completion (success or failure).
    pub fn commit_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id).ok_or_else(|| {
                // Release lock even if transaction not found
                self.release_write_lock(tx_id);
                crate::error::IronBaseError::TransactionAborted(format!(
                    "Transaction {} not found",
                    tx_id
                ))
            })?
        };

        // Commit through storage engine
        let mut result = {
            let mut storage = self.storage.write();
            storage.commit_transaction(&mut transaction)
        };

        // Bring the in-memory indexes up to date while the write lock is still
        // held, so no other writer or transaction sees them stale (audit
        // 2026-10-06 #11).
        if result.is_ok() {
            result = self.apply_committed_transaction_to_indexes(&transaction);
        }

        // Advance watermark on successful commit (for WAL-replay index recovery)
        if result.is_ok() {
            self.max_committed_tx_id
                .fetch_max(tx_id, std::sync::atomic::Ordering::SeqCst);
        }

        // Always release write lock (even on error to prevent deadlock)
        self.release_write_lock(tx_id);

        result
    }

    /// Apply a committed transaction's operations to the in-memory indexes of
    /// every collection it touched.
    fn apply_committed_transaction_to_indexes(&self, transaction: &Transaction) -> Result<()> {
        let mut collections: Vec<&str> = Vec::new();
        for op in transaction.operations() {
            let name = match op {
                crate::transaction::Operation::Insert { collection, .. }
                | crate::transaction::Operation::Update { collection, .. }
                | crate::transaction::Operation::Delete { collection, .. } => collection.as_str(),
            };
            if !collections.contains(&name) {
                collections.push(name);
            }
        }
        for name in collections {
            self.collection(name)?
                .apply_committed_ops_to_indexes(transaction.operations())?;
        }
        Ok(())
    }

    /// Rollback a transaction (discard all buffered operations) - StorageEngine-specific
    ///
    /// Automatically releases the write lock on completion (success or failure).
    pub fn rollback_transaction(&self, tx_id: TransactionId) -> Result<()> {
        // Remove transaction from active list
        let mut transaction = {
            let mut active = self.active_transactions.write();
            active.remove(&tx_id).ok_or_else(|| {
                // Release lock even if transaction not found
                self.release_write_lock(tx_id);
                crate::error::IronBaseError::TransactionAborted(format!(
                    "Transaction {} not found",
                    tx_id
                ))
            })?
        };

        // Rollback through storage engine
        let result = {
            let mut storage = self.storage.write();
            storage.rollback_transaction(&mut transaction)
        };

        // Always release write lock (even on error to prevent deadlock)
        self.release_write_lock(tx_id);

        result
    }

    // ========== Two-Phase Commit Helper Methods (StorageEngine-specific) ==========

    /// Construct index file path for a collection's index
    /// Format: {db_path_without_ext}.{index_name}.idx
    ///
    /// Example: "/data/myapp.mlite" + "users_age" → "/data/myapp.users_age.idx"
    #[cfg(test)]
    pub(crate) fn get_index_file_path(
        &self,
        _collection_name: &str,
        index_name: &str,
    ) -> std::path::PathBuf {
        use std::path::PathBuf;

        let mut path = PathBuf::from(&self.db_path);

        // Remove .mlite extension if present
        if path.extension().map(|e| e == "mlite").unwrap_or(false) {
            path.set_extension("");
        }

        // Append index name and .idx extension
        let index_file = format!("{}.{}.idx", path.display(), index_name);
        PathBuf::from(index_file)
    }

    /// Extract collection name from transaction's first operation
    #[cfg(test)]
    pub(crate) fn get_collection_from_transaction(transaction: &Transaction) -> Option<String> {
        transaction.operations().first().map(|op| match op {
            crate::transaction::Operation::Insert { collection, .. } => collection.clone(),
            crate::transaction::Operation::Update { collection, .. } => collection.clone(),
            crate::transaction::Operation::Delete { collection, .. } => collection.clone(),
        })
    }

    /// Count documents matching a query.
    ///
    /// This method is provided for FFI consistency - it ensures the same CollectionCore
    /// instance is used for both write and read operations, avoiding cache inconsistencies.
    ///
    /// # Arguments
    /// * `collection_name` - Name of the collection
    /// * `query` - Query filter as JSON value
    ///
    /// # Returns
    /// Number of matching documents
    pub fn count_documents(&self, collection_name: &str, query: &Value) -> Result<u64> {
        self.check_not_closed()?;
        let collection = self.get_collection(collection_name)?;
        collection.count_documents(query)
    }

    /// Find documents matching a query.
    ///
    /// This method is provided for FFI consistency - it ensures the same CollectionCore
    /// instance is used for both write and read operations, avoiding cache inconsistencies.
    ///
    /// # Arguments
    /// * `collection_name` - Name of the collection
    /// * `query` - Query filter as JSON value
    ///
    /// # Returns
    /// Vector of matching documents
    pub fn find(&self, collection_name: &str, query: &Value) -> Result<Vec<Value>> {
        self.check_not_closed()?;
        let collection = self.get_collection(collection_name)?;
        collection.find(query)
    }

    /// Find a single document matching a query.
    ///
    /// This method is provided for FFI consistency.
    ///
    /// # Arguments
    /// * `collection_name` - Name of the collection
    /// * `query` - Query filter as JSON value
    ///
    /// # Returns
    /// The first matching document, or None
    pub fn find_one(&self, collection_name: &str, query: &Value) -> Result<Option<Value>> {
        self.check_not_closed()?;
        let collection = self.get_collection(collection_name)?;
        collection.find_one(query)
    }
}

// ============================================================================
// GENERIC TRANSACTION IMPLEMENTATION (all storage backends)
// ============================================================================

impl<S: Storage + RawStorage> DatabaseCore<S> {
    // ========== Transaction Write Lock (Read Committed Isolation) ==========

    /// Acquire exclusive write lock for a transaction
    ///
    /// Only one transaction can hold the write lock at a time.
    /// This provides Read Committed isolation - prevents dirty reads.
    ///
    /// Uses default timeout of 5 seconds.
    ///
    /// # Errors
    /// Returns error if lock cannot be acquired within timeout.
    pub fn acquire_write_lock(&self, tx_id: TransactionId) -> Result<()> {
        self.acquire_write_lock_with_timeout(tx_id, std::time::Duration::from_secs(5))
    }

    /// Acquire exclusive write lock for a transaction with custom timeout.
    ///
    /// If another transaction holds the lock, this will wait up to the specified
    /// timeout before returning an error.
    ///
    /// # Arguments
    /// * `tx_id` - The transaction requesting the lock
    /// * `timeout` - Maximum time to wait for the lock
    ///
    /// # Errors
    /// Returns error if lock cannot be acquired within timeout.
    pub fn acquire_write_lock_with_timeout(
        &self,
        tx_id: TransactionId,
        timeout: std::time::Duration,
    ) -> Result<()> {
        let mut lock = self.write_transaction_lock.lock();
        if lock.holder == Some(tx_id) {
            // This transaction already holds the lock - OK
            return Ok(());
        }

        lock.tx_waiting += 1;
        let acquired = loop {
            // Free = no other transaction and no auto-commit write in progress
            if lock.holder.is_none() && lock.auto_writers == 0 {
                lock.holder = Some(tx_id);
                break Ok(());
            }

            // Wait for a release (transaction or auto-commit write), or timeout
            let wait_result = self.write_lock_condvar.wait_for(&mut lock, timeout);
            if wait_result.timed_out() {
                break Err(IronBaseError::TransactionAborted(format!(
                    "Timeout waiting for write lock after {:?}. Lock held by {}.",
                    timeout,
                    match lock.holder {
                        Some(h) => format!("transaction {}", h),
                        None => format!("{} auto-commit write(s)", lock.auto_writers),
                    }
                )));
            }
        };
        lock.tx_waiting -= 1;
        if acquired.is_err() {
            // Auto-commits blocked on tx_waiting may proceed now
            self.write_lock_condvar.notify_all();
        }
        drop(lock);

        if acquired.is_ok() {
            // Mark transaction as holding write lock
            let mut active = self.active_transactions.write();
            if let Some(tx) = active.get_mut(&tx_id) {
                tx.mark_write_lock_acquired();
            }
        }
        acquired
    }

    /// Release the write lock held by a transaction
    ///
    /// Called automatically on commit or rollback.
    /// Safe to call even if transaction doesn't hold the lock.
    pub fn release_write_lock(&self, tx_id: TransactionId) {
        let mut lock = self.write_transaction_lock.lock();
        if lock.holder == Some(tx_id) {
            lock.holder = None;
            // Wake every waiter: both transactions and auto-commit writes wait here
            self.write_lock_condvar.notify_all();
        }
    }

    /// Check if a transaction currently holds the write lock
    pub fn holds_write_lock(&self, tx_id: TransactionId) -> bool {
        let lock = self.write_transaction_lock.lock();
        lock.holder == Some(tx_id)
    }

    /// Check if any transaction holds the write lock (for auto-commit conflict check)
    pub fn has_active_write_transaction(&self) -> bool {
        let lock = self.write_transaction_lock.lock();
        lock.holder.is_some()
    }

    /// Get the ID of the transaction holding the write lock, if any
    pub fn get_write_lock_holder(&self) -> Option<TransactionId> {
        let lock = self.write_transaction_lock.lock();
        lock.holder
    }

    /// Enter an auto-commit write (shared hold of the write lock).
    ///
    /// Waits while an explicit transaction holds the lock or is waiting for
    /// it, then holds the lock shared until the returned guard is dropped.
    /// The guard must live for the whole read-modify-write, so a transaction
    /// cannot take a snapshot in the middle of it (audit 2026-10-06 #14).
    /// Uses a default timeout of 5 seconds.
    pub(crate) fn enter_auto_write(&self) -> Result<super::AutoWriteGuard<'_>> {
        self.enter_auto_write_with_timeout(std::time::Duration::from_secs(5))
    }

    /// `enter_auto_write` with a custom timeout
    pub(crate) fn enter_auto_write_with_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<super::AutoWriteGuard<'_>> {
        let mut lock = self.write_transaction_lock.lock();

        loop {
            if lock.holder.is_none() && lock.tx_waiting == 0 {
                lock.auto_writers += 1;
                return Ok(super::AutoWriteGuard {
                    state: &self.write_transaction_lock,
                    condvar: &self.write_lock_condvar,
                });
            }

            // Wait for release_write_lock() to notify us, or timeout
            let wait_result = self.write_lock_condvar.wait_for(&mut lock, timeout);
            if wait_result.timed_out() {
                return Err(IronBaseError::TransactionAborted(format!(
                    "Timeout waiting for write transaction to complete after {:?}. Lock held by transaction {}.",
                    timeout,
                    lock.holder.map_or("unknown".to_string(), |h| h.to_string())
                )));
            }
            // Condvar woke us — loop back to check if lock is now free
        }
    }

    // ========== ACD Transaction API ==========

    /// Begin a new transaction
    /// Returns the transaction ID
    pub fn begin_transaction(&self) -> TransactionId {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::SeqCst);
        let transaction = Transaction::new(tx_id);

        let mut active = self.active_transactions.write();
        active.insert(tx_id, transaction);

        tx_id
    }

    /// Get a reference to an active transaction (for adding operations)
    pub fn get_transaction(&self, tx_id: TransactionId) -> Option<Transaction> {
        let active = self.active_transactions.read();
        active.get(&tx_id).cloned()
    }

    /// Update a transaction (after adding operations)
    pub fn update_transaction(&self, tx_id: TransactionId, transaction: Transaction) -> Result<()> {
        let mut active = self.active_transactions.write();
        active.insert(tx_id, transaction);
        Ok(())
    }

    /// Execute a closure with mutable access to a transaction
    /// This is more efficient than get + modify + update
    pub fn with_transaction<F, R>(&self, tx_id: TransactionId, f: F) -> Result<R>
    where
        F: FnOnce(&mut Transaction) -> Result<R>,
    {
        let mut active = self.active_transactions.write();
        let transaction = active.get_mut(&tx_id).ok_or_else(|| {
            crate::error::IronBaseError::TransactionAborted(format!(
                "Transaction {} not found",
                tx_id
            ))
        })?;

        f(transaction)
    }

    // ========== Transaction Convenience Methods ==========

    /// Insert one document within a transaction (convenience method)
    ///
    /// This is a helper that combines collection lookup and transaction execution.
    /// Equivalent to: db.collection(name).insert_one_tx(doc, tx)
    ///
    /// # Write Lock
    /// Automatically acquires exclusive write lock on first write operation.
    /// Only one write transaction can be active at a time (Read Committed isolation).
    pub fn insert_one_tx(
        &self,
        collection_name: &str,
        document: HashMap<String, Value>,
        tx_id: TransactionId,
    ) -> Result<DocumentId> {
        // Acquire write lock (idempotent if already held)
        self.acquire_write_lock(tx_id)?;

        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.insert_one_tx(document, transaction)
        })
    }

    /// Update one document within a transaction (convenience method)
    ///
    /// Returns (matched_count, modified_count)
    ///
    /// # Write Lock
    /// Automatically acquires exclusive write lock on first write operation.
    /// Only one write transaction can be active at a time (Read Committed isolation).
    pub fn update_one_tx(
        &self,
        collection_name: &str,
        query: &Value,
        update: Value,
        tx_id: TransactionId,
    ) -> Result<(u64, u64)> {
        // Acquire write lock (idempotent if already held)
        self.acquire_write_lock(tx_id)?;

        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.update_one_tx(query, update, transaction)
        })
    }

    /// Delete one document within a transaction (convenience method)
    ///
    /// Returns deleted_count
    ///
    /// # Write Lock
    /// Automatically acquires exclusive write lock on first write operation.
    /// Only one write transaction can be active at a time (Read Committed isolation).
    pub fn delete_one_tx(
        &self,
        collection_name: &str,
        query: &Value,
        tx_id: TransactionId,
    ) -> Result<u64> {
        // Acquire write lock (idempotent if already held)
        self.acquire_write_lock(tx_id)?;

        let collection = self.collection(collection_name)?;

        self.with_transaction(tx_id, |transaction| {
            collection.delete_one_tx(query, transaction)
        })
    }
}
