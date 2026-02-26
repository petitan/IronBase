//! Job types for async operations

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

/// Unique job identifier
pub type JobId = String;

/// Job status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status")]
pub enum JobStatus {
    /// Job is waiting to be processed
    #[serde(rename = "pending")]
    Pending,

    /// Job is currently running
    #[serde(rename = "running")]
    Running {
        /// Progress percentage (0.0 - 100.0)
        progress: f64,
        /// Current status message
        message: String,
    },

    /// Job completed successfully
    #[serde(rename = "completed")]
    Completed {
        /// Job result
        result: Value,
    },

    /// Job failed with error
    #[serde(rename = "failed")]
    Failed {
        /// Error message
        error: String,
    },

    /// Job was cancelled
    #[serde(rename = "cancelled")]
    Cancelled,
}

impl JobStatus {
    /// Check if job is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobStatus::Completed { .. } | JobStatus::Failed { .. } | JobStatus::Cancelled
        )
    }

    /// Check if job is running
    pub fn is_running(&self) -> bool {
        matches!(self, JobStatus::Running { .. })
    }
}

/// Job type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobType {
    /// Backfill embeddings for existing documents
    EmbedBackfill {
        collection: String,
        provider: String,
    },
    /// Import and embed a document with chunking
    EmbedDocument { collection: String, doc_id: String },
    /// Database compaction (background, non-blocking)
    Compact,
    /// Custom job type
    Custom { name: String },
}

/// Job information
#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    /// Unique job identifier
    pub id: JobId,
    /// Job type
    pub job_type: JobType,
    /// Current status
    pub status: JobStatus,
    /// When the job was created (seconds since epoch for serialization)
    pub created_at_secs: u64,
    /// When the job was last updated (seconds since epoch for serialization)
    pub updated_at_secs: u64,
    /// Items processed so far
    pub items_processed: usize,
    /// Total items to process (if known)
    pub items_total: Option<usize>,
}

/// Internal job state (not serialized directly)
pub struct Job {
    pub info: JobInfo,
    /// When the job was created
    pub created_at: Instant,
    /// When the job was last updated
    pub updated_at: Instant,
    /// Cancel flag
    pub cancelled: bool,
}

impl Job {
    /// Create a new job
    pub fn new(id: JobId, job_type: JobType) -> Self {
        let now = Instant::now();
        Self {
            info: JobInfo {
                id,
                job_type,
                status: JobStatus::Pending,
                created_at_secs: 0, // Will be updated on serialization
                updated_at_secs: 0,
                items_processed: 0,
                items_total: None,
            },
            created_at: now,
            updated_at: now,
            cancelled: false,
        }
    }

    /// Update job status
    pub fn set_status(&mut self, status: JobStatus) {
        self.info.status = status;
        self.updated_at = Instant::now();
    }

    /// Update progress
    pub fn update_progress(&mut self, processed: usize, total: Option<usize>, message: &str) {
        self.info.items_processed = processed;
        self.info.items_total = total;

        let progress = match total {
            Some(t) if t > 0 => (processed as f64 / t as f64) * 100.0,
            _ => 0.0,
        };

        self.info.status = JobStatus::Running {
            progress,
            message: message.to_string(),
        };
        self.updated_at = Instant::now();
    }

    /// Mark job as completed
    pub fn complete(&mut self, result: Value) {
        self.info.status = JobStatus::Completed { result };
        self.updated_at = Instant::now();
    }

    /// Mark job as failed
    pub fn fail(&mut self, error: String) {
        self.info.status = JobStatus::Failed { error };
        self.updated_at = Instant::now();
    }

    /// Mark job as cancelled
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.info.status = JobStatus::Cancelled;
        self.updated_at = Instant::now();
    }

    /// Check if job was cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Get serializable info
    pub fn to_info(&self) -> JobInfo {
        let mut info = self.info.clone();
        info.created_at_secs = self.created_at.elapsed().as_secs();
        info.updated_at_secs = self.updated_at.elapsed().as_secs();
        info
    }
}
