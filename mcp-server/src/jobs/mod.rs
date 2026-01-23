//! Background job management for async operations
//!
//! Provides a job manager for tracking long-running operations like:
//! - Backfilling embeddings for existing documents
//! - Processing large document imports
//! - Reindexing operations

pub mod manager;
pub mod types;

pub use manager::JobManager;
pub use types::{Job, JobId, JobInfo, JobStatus, JobType};
