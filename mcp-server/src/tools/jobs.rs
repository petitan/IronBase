//! Job management tool handlers

use crate::error::{McpError, Result};
use crate::jobs::JobManager;
use ironbase_mcp_types::responses::{
    EmbedJobCancelResponse, EmbedJobListResponse, EmbedJobStatusResponse, Flag,
};
use serde_json::Value;
use std::sync::Arc;

use super::params::ParseParams;

// Parameter structs: shared with clients, re-exported under their old path.
pub use ironbase_mcp_types::params::{JobCancelParams, JobListParams, JobStatusParams};

// ============================================================================
// Dispatch
// ============================================================================

/// Dispatch job tool calls
pub fn dispatch(name: &str, params: Value, job_manager: &Option<Arc<JobManager>>) -> Result<Value> {
    let manager = job_manager
        .as_ref()
        .ok_or_else(|| McpError::internal("Job manager not available"))?;

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

    let response = match manager.get_job_info(&p.job_id) {
        Some(job) => EmbedJobStatusResponse::Found { found: Flag, job },
        None => EmbedJobStatusResponse::NotFound {
            found: Flag,
            message: format!("Job '{}' not found", p.job_id),
        },
    };
    Ok(serde_json::to_value(response)?)
}

fn handle_job_list(params: Value, manager: &JobManager) -> Result<Value> {
    let p: JobListParams = JobListParams::parse(params)?;

    let jobs = if p.active_only {
        manager.list_jobs_by_status(true) // Only running jobs
    } else {
        manager.list_jobs()
    };

    let active_count = manager.active_job_count();

    Ok(serde_json::to_value(EmbedJobListResponse {
        count: jobs.len(),
        jobs,
        active_count,
    })?)
}

fn handle_job_cancel(params: Value, manager: &JobManager) -> Result<Value> {
    let p: JobCancelParams = JobCancelParams::parse(params)?;

    let response = if manager.cancel_job(&p.job_id) {
        EmbedJobCancelResponse {
            success: true,
            message: format!("Job '{}' cancelled", p.job_id),
            job_id: p.job_id,
            status: None,
        }
    } else {
        // Check if job exists but is already completed
        match manager
            .get_job_info(&p.job_id)
            .filter(|info| info.status.is_terminal())
        {
            Some(info) => EmbedJobCancelResponse {
                success: false,
                message: format!("Job '{}' is already in terminal state", p.job_id),
                job_id: p.job_id,
                status: Some(info.status),
            },
            None => EmbedJobCancelResponse {
                success: false,
                message: format!("Job '{}' not found", p.job_id),
                job_id: p.job_id,
                status: None,
            },
        }
    };
    Ok(serde_json::to_value(response)?)
}
