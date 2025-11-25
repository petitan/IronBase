// ironbase-core/src/lib.rs
// Pure Rust API - NO Python/PyO3 dependencies

pub mod aggregation;
pub mod btree;
pub mod catalog_serde;
pub mod collection_core;
pub mod database;
pub mod document;
pub mod durability;
pub mod error;
pub mod find_options;
pub mod index;
pub mod logging;
pub mod query;
pub mod query_cache;
pub mod query_planner;
pub mod storage;
pub mod transaction;
pub mod wal;

#[cfg(test)]
mod test_auto_commit;
#[cfg(test)]
mod transaction_benchmarks;
#[cfg(test)]
mod transaction_integration_tests;
#[cfg(test)]
mod transaction_property_tests;

// Public exports
pub use collection_core::{CollectionCore, FindCursor, InsertManyResult};
pub use database::DatabaseCore;
pub use document::{Document, DocumentId};
pub use durability::DurabilityMode;
pub use error::{MongoLiteError, Result};
pub use find_options::FindOptions;
pub use logging::{get_log_level, set_log_level, LogLevel};
pub use query::Query;
pub use query_cache::{CacheStats, QueryCache, QueryHash};
pub use storage::{CompactionStats, StorageEngine};
pub use transaction::{Operation, Transaction, TransactionId, TransactionState};
pub use wal::{WALEntry, WALEntryType, WriteAheadLog};
