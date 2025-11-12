// ironbase-core/src/lib.rs
// Pure Rust API - NO Python/PyO3 dependencies

pub mod error;
pub mod document;
pub mod storage;
pub mod query;
pub mod query_cache;
pub mod index;
pub mod btree;
pub mod query_planner;
pub mod aggregation;
pub mod find_options;
pub mod collection_core;
pub mod database;
pub mod transaction;
pub mod wal;
pub mod catalog_serde;

#[cfg(test)]
mod transaction_property_tests;
#[cfg(test)]
mod transaction_integration_tests;
#[cfg(test)]
mod transaction_benchmarks;

// Public exports
pub use error::{MongoLiteError, Result};
pub use document::{Document, DocumentId};
pub use storage::{StorageEngine, CompactionStats};
pub use query::Query;
pub use query_cache::{QueryCache, QueryHash, CacheStats};
pub use find_options::FindOptions;
pub use collection_core::{CollectionCore, InsertManyResult};
pub use database::DatabaseCore;
pub use transaction::{Transaction, TransactionId, TransactionState, Operation};
pub use wal::{WriteAheadLog, WALEntry, WALEntryType};
