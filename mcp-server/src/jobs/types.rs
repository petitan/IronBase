//! Job types for async operations

use serde_json::Value;
use std::time::Instant;

// The reported job types: shared with clients, re-exported under their old path.
pub use ironbase_mcp_types::responses::{JobId, JobInfo, JobStatus, JobType};

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

    /// Update job status (no-op once terminal)
    pub fn set_status(&mut self, status: JobStatus) {
        if self.info.status.is_terminal() {
            return;
        }
        self.info.status = status;
        self.updated_at = Instant::now();
    }

    /// Update progress
    ///
    /// Keeps a terminal status: a worker reporting after a cancel turned the
    /// job back into Running for good (MCP audit 2026-10-14 M3). Only the
    /// item counters are updated then.
    pub fn update_progress(&mut self, processed: usize, total: Option<usize>, message: &str) {
        // The counters stay accurate (a batch finished after the cancel)
        self.info.items_processed = processed;
        self.info.items_total = total;
        self.updated_at = Instant::now();
        if self.info.status.is_terminal() {
            return;
        }

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

    /// Mark job as completed (no-op once terminal, e.g. cancelled)
    pub fn complete(&mut self, result: Value) {
        if self.info.status.is_terminal() {
            return;
        }
        self.info.status = JobStatus::Completed { result };
        self.updated_at = Instant::now();
    }

    /// Mark job as failed (no-op once terminal, e.g. cancelled)
    pub fn fail(&mut self, error: String) {
        if self.info.status.is_terminal() {
            return;
        }
        self.info.status = JobStatus::Failed { error };
        self.updated_at = Instant::now();
    }

    /// Mark job as cancelled (no-op once terminal)
    pub fn cancel(&mut self) {
        if self.info.status.is_terminal() {
            return;
        }
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
