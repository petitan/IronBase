//! Job management tool handlers

use crate::error::{McpError, Result};
use crate::jobs::JobManager;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use super::params::ParseParams;

// ============================================================================
// Parameter Structs
// ============================================================================

/// Parameters for `embed_job_status` tool
#[derive(Debug, Deserialize)]
pub struct JobStatusParams {
    pub job_id: String,
}

/// Parameters for `embed_job_list` tool
#[derive(Debug, Deserialize)]
pub struct JobListParams {
    #[serde(default)]
    pub active_only: bool,
}

/// Parameters for `embed_job_cancel` tool
#[derive(Debug, Deserialize)]
pub struct JobCancelParams {
    pub job_id: String,
}

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch job tool calls
pub fn dispatch(
    name: &str,
    params: Value,
    job_manager: &Option<Arc<JobManager>>,
) -> Result<Value> {
    let manager = job_manager.as_ref().ok_or_else(|| {
        McpError::internal("Job manager not available")
    })?;

    match name {
        "embed_job_status" => handle_job_status(params, manager),
        "embed_job_list" => handle_job_list(params, manager),
        "embed_job_cancel" => handle_job_cancel(params, manager),
        _ => Err(McpError::invalid_params(format!(
            "Unknown job tool: {}",
            name
        ))),
    }
}

// ============================================================================
// Tool Handlers
// ============================================================================

fn handle_job_status(params: Value, manager: &JobManager) -> Result<Value> {
    let p: JobStatusParams = JobStatusParams::parse(params)?;

    match manager.get_job_info(&p.job_id) {
        Some(info) => Ok(json!({
            "found": true,
            "job": info
        })),
        None => Ok(json!({
            "found": false,
            "message": format!("Job '{}' not found", p.job_id)
        })),
    }
}

fn handle_job_list(params: Value, manager: &JobManager) -> Result<Value> {
    let p: JobListParams = JobListParams::parse(params)?;

    let jobs = if p.active_only {
        manager.list_jobs_by_status(true) // Only running jobs
    } else {
        manager.list_jobs()
    };

    let active_count = manager.active_job_count();

    Ok(json!({
        "jobs": jobs,
        "count": jobs.len(),
        "active_count": active_count
    }))
}

fn handle_job_cancel(params: Value, manager: &JobManager) -> Result<Value> {
    let p: JobCancelParams = JobCancelParams::parse(params)?;

    if manager.cancel_job(&p.job_id) {
        Ok(json!({
            "success": true,
            "job_id": p.job_id,
            "message": format!("Job '{}' cancelled", p.job_id)
        }))
    } else {
        // Check if job exists but is already completed
        if let Some(info) = manager.get_job_info(&p.job_id) {
            if info.status.is_terminal() {
                return Ok(json!({
                    "success": false,
                    "job_id": p.job_id,
                    "message": format!("Job '{}' is already in terminal state", p.job_id),
                    "status": info.status
                }));
            }
        }

        Ok(json!({
            "success": false,
            "job_id": p.job_id,
            "message": format!("Job '{}' not found", p.job_id)
        }))
    }
}
