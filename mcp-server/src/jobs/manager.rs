//! Job manager for background async operations

use super::types::{Job, JobId, JobInfo, JobType};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum number of completed jobs to keep in history
const MAX_COMPLETED_JOBS: usize = 100;

/// Job manager for tracking and managing async operations
pub struct JobManager {
    /// Active and recent jobs
    jobs: RwLock<HashMap<JobId, Arc<RwLock<Job>>>>,
    /// Counter for generating unique job IDs
    next_id: RwLock<u64>,
}

impl JobManager {
    /// Create a new job manager
    pub fn new() -> Self {
        Self {
            jobs: RwLock::new(HashMap::new()),
            next_id: RwLock::new(1),
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
        self.jobs.read().get(id).map(|j| j.read().to_info())
    }

    /// List all jobs
    pub fn list_jobs(&self) -> Vec<JobInfo> {
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

    /// Cleanup old completed jobs to prevent memory leak
    fn cleanup_completed_jobs(&self, jobs: &mut HashMap<JobId, Arc<RwLock<Job>>>) {
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

        if completed.len() > MAX_COMPLETED_JOBS {
            // Sort by updated_at (oldest first)
            let mut to_remove: Vec<_> = completed;
            to_remove.sort_by_key(|(_, updated_at)| *updated_at);

            // Remove oldest completed jobs
            let remove_count = to_remove.len() - MAX_COMPLETED_JOBS;
            for (id, _) in to_remove.into_iter().take(remove_count) {
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
}
