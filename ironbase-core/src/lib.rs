// ironbase-core/src/lib.rs
// Pure Rust API - NO Python/PyO3 dependencies

// Allow clippy lints that are too strict for this codebase
#![allow(clippy::too_many_arguments)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::only_used_in_recursion)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::nonminimal_bool)]
#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::redundant_comparisons)]
#![allow(clippy::suspicious_open_options)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::result_large_err)]
#![allow(clippy::match_result_ok)]
#![allow(clippy::manual_unwrap_or_default)]
#![allow(clippy::manual_unwrap_or)]
#![allow(clippy::single_match)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::approx_constant)]
// Tests may have helper functions not used in all test cases
#![cfg_attr(test, allow(dead_code))]
#![cfg_attr(test, allow(unused_variables))]

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
pub mod recovery;
pub mod storage;
pub mod transaction;
pub mod value_utils;
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
pub use recovery::{
    IndexOperation, IndexReplay, IndexReplayStats, OperationReplay, RecoveredIndexChange,
    RecoveryCoordinator, RecoveryStats, ReplayStats,
};
pub use storage::{CompactionStats, StorageEngine};
pub use transaction::{Operation, Transaction, TransactionId, TransactionState};
pub use wal::{
    CommittedTransaction, TransactionGrouper, WALEntry, WALEntryIterator, WALEntryType,
    WriteAheadLog,
};
