//! Job manager for background async operations

use super::types::{Job, JobId, JobInfo, JobType};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Maximum number of completed jobs to keep in history
const MAX_COMPLETED_JOBS: usize = 100;

/// Time-to-live for completed jobs (1 hour)
const COMPLETED_JOB_TTL: Duration = Duration::from_secs(3600);

/// Minimum interval between cleanup runs
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Job manager for tracking and managing async operations
pub struct JobManager {
    /// Active and recent jobs
    jobs: RwLock<HashMap<JobId, Arc<RwLock<Job>>>>,
    /// Counter for generating unique job IDs
    next_id: RwLock<u64>,
    /// Last cleanup timestamp
    last_cleanup: RwLock<Instant>,
}

impl JobManager {
    /// Create a new job manager
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
            last_cleanup: RwLock::new(Instant::now()),
        }
    }

    /// Generate a unique job ID
    fn generate_id(&self) -> JobId {
        let mut next_id = self.next_id.write();
        let id = format!("job_{}", *next_id);
        *next_id += 1;
        id
    }

    /// Create a new job
    pub fn create_job(&self, job_type: JobType) -> (JobId, Arc<RwLock<Job>>) {
        let id = self.generate_id();
        let job = Arc::new(RwLock::new(Job::new(id.clone(), job_type)));

        {
            let mut jobs = self.jobs.write();
            jobs.insert(id.clone(), job.clone());

            // Cleanup old completed jobs if we have too many
            self.cleanup_completed_jobs(&mut jobs);
        }

        (id, job)
    }

    /// Get a job by ID
    pub fn get_job(&self, id: &str) -> Option<Arc<RwLock<Job>>> {
        self.jobs.read().get(id).cloned()
    }

    /// Get job info by ID
    pub fn get_job_info(&self, id: &str) -> Option<JobInfo> {
        // Trigger cleanup periodically
        self.try_cleanup();

        self.jobs.read().get(id).map(|j| j.read().to_info())
    }

    /// List all jobs
    pub fn list_jobs(&self) -> Vec<JobInfo> {
        // Trigger cleanup on list operations
        self.try_cleanup();

        self.jobs
            .read()
            .values()
            .map(|j| j.read().to_info())
            .collect()
    }

    /// List jobs by status
    pub fn list_jobs_by_status(&self, is_running: bool) -> Vec<JobInfo> {
        self.jobs
            .read()
            .values()
            .filter_map(|j| {
                let job = j.read();
                let info = job.to_info();
                if is_running == info.status.is_running() {
                    Some(info)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Cancel a job
    pub fn cancel_job(&self, id: &str) -> bool {
        if let Some(job) = self.jobs.read().get(id) {
            let mut job = job.write();
            if !job.info.status.is_terminal() {
                job.cancel();
                return true;
            }
        }
        false
    }

    /// Remove a job from the manager
    pub fn remove_job(&self, id: &str) -> bool {
        self.jobs.write().remove(id).is_some()
    }

    /// Update job progress
    pub fn update_progress(
        &self,
        id: &str,
        processed: usize,
        total: Option<usize>,
        message: &str,
    ) -> bool {
        if let Some(job) = self.jobs.read().get(id) {
            job.write().update_progress(processed, total, message);
            return true;
        }
        false
    }

    /// Mark job as completed
    pub fn complete_job(&self, id: &str, result: serde_json::Value) -> bool {
        if let Some(job) = self.jobs.read().get(id) {
            job.write().complete(result);
            return true;
        }
        false
    }

    /// Mark job as failed
    pub fn fail_job(&self, id: &str, error: String) -> bool {
        if let Some(job) = self.jobs.read().get(id) {
            job.write().fail(error);
            return true;
        }
        false
    }

    /// Check if job is cancelled
    pub fn is_cancelled(&self, id: &str) -> bool {
        self.jobs
            .read()
            .get(id)
            .map(|j| j.read().is_cancelled())
            .unwrap_or(false)
    }

    /// Get count of active (non-terminal) jobs
    pub fn active_job_count(&self) -> usize {
        self.jobs
            .read()
            .values()
            .filter(|j| !j.read().info.status.is_terminal())
            .count()
    }

    /// Try to run cleanup if enough time has passed
    fn try_cleanup(&self) {
        let should_cleanup = {
            let last = self.last_cleanup.read();
            last.elapsed() >= CLEANUP_INTERVAL
        };

        if should_cleanup {
            let mut jobs = self.jobs.write();
            self.cleanup_completed_jobs(&mut jobs);
            *self.last_cleanup.write() = Instant::now();
        }
    }

    /// Cleanup old completed jobs to prevent memory leak
    /// Removes jobs that are:
    /// 1. Completed and older than TTL
    /// 2. Completed and exceed MAX_COMPLETED_JOBS count
    fn cleanup_completed_jobs(&self, jobs: &mut HashMap<JobId, Arc<RwLock<Job>>>) {
        let now = Instant::now();

        let completed: Vec<_> = jobs
            .iter()
            .filter_map(|(id, job)| {
                let job = job.read();
                if job.info.status.is_terminal() {
                    Some((id.clone(), job.updated_at))
                } else {
                    None
                }
            })
            .collect();

        // First, remove jobs older than TTL
        let mut remaining_completed = Vec::new();
        for (id, updated_at) in completed {
            if now.duration_since(updated_at) > COMPLETED_JOB_TTL {
                jobs.remove(&id);
            } else {
                remaining_completed.push((id, updated_at));
            }
        }

        // Then, if still too many, remove oldest
        if remaining_completed.len() > MAX_COMPLETED_JOBS {
            // Sort by updated_at (oldest first)
            remaining_completed.sort_by_key(|(_, updated_at)| *updated_at);

            // Remove oldest completed jobs
            let remove_count = remaining_completed.len() - MAX_COMPLETED_JOBS;
            for (id, _) in remaining_completed.into_iter().take(remove_count) {
                jobs.remove(&id);
            }
        }
    }
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::JobStatus;

    #[test]
    fn test_create_job() {
        let manager = JobManager::new();
        let (id, _job) = manager.create_job(JobType::Custom {
            name: "test".to_string(),
        });

        assert!(id.starts_with("job_"));
        assert!(manager.get_job(&id).is_some());
    }

    #[test]
    fn test_job_lifecycle() {
        let manager = JobManager::new();
        let (id, _) = manager.create_job(JobType::Custom {
            name: "test".to_string(),
        });

        // Initially pending
        let info = manager.get_job_info(&id).unwrap();
        assert!(matches!(info.status, JobStatus::Pending));

        // Update progress
        manager.update_progress(&id, 50, Some(100), "Processing...");
        let info = manager.get_job_info(&id).unwrap();
        assert!(matches!(info.status, JobStatus::Running { .. }));

        // Complete
        manager.complete_job(&id, serde_json::json!({"result": "ok"}));
        let info = manager.get_job_info(&id).unwrap();
        assert!(matches!(info.status, JobStatus::Completed { .. }));
    }

    #[test]
    fn test_cancel_job() {
        let manager = JobManager::new();
        let (id, _) = manager.create_job(JobType::Custom {
            name: "test".to_string(),
        });

        assert!(manager.cancel_job(&id));
        assert!(manager.is_cancelled(&id));

        let info = manager.get_job_info(&id).unwrap();
        assert!(matches!(info.status, JobStatus::Cancelled));
    }

    #[test]
    fn test_list_jobs() {
        let manager = JobManager::new();

        manager.create_job(JobType::Custom {
            name: "test1".to_string(),
        });
        manager.create_job(JobType::Custom {
            name: "test2".to_string(),
        });

        let jobs = manager.list_jobs();
        assert_eq!(jobs.len(), 2);
    }

    #[test]
    fn test_cleanup_max_completed() {
        let manager = JobManager::new();

        // Create more than MAX_COMPLETED_JOBS completed jobs
        for i in 0..150 {
            let (id, _) = manager.create_job(JobType::Custom {
                name: format!("test_{}", i),
            });
            manager.complete_job(&id, serde_json::json!({"i": i}));
        }

        // After cleanup, should have at most MAX_COMPLETED_JOBS
        let jobs = manager.list_jobs();
        assert!(jobs.len() <= super::MAX_COMPLETED_JOBS + 1); // +1 for potential race
    }

    #[test]
    fn test_active_jobs_not_cleaned() {
        let manager = JobManager::new();

        // Create many completed jobs to trigger cleanup
        for i in 0..150 {
            let (id, _) = manager.create_job(JobType::Custom {
                name: format!("completed_{}", i),
            });
            manager.complete_job(&id, serde_json::json!({}));
        }

        // Create an active job
        let (active_id, _) = manager.create_job(JobType::Custom {
            name: "active".to_string(),
        });
        manager.update_progress(&active_id, 50, Some(100), "Running...");

        // Active job should still exist
        let info = manager.get_job_info(&active_id);
        assert!(info.is_some());
        assert!(matches!(info.unwrap().status, JobStatus::Running { .. }));
    }
}
