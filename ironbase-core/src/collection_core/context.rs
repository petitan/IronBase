//! Query execution context for find operations

use serde_json::Value;
use std::collections::HashMap;

/// Query execution context extracted from FindOptions
/// Single Responsibility: Transform user options into execution strategy
///
/// ## Deadline Propagation (v1.0.182+)
///
/// The `cancel_flag` and `deadline` fields enable cooperative cancellation:
/// - Propagated through: `collect_doc_ids_with_options` → `collect_doc_ids_from_plan`
///   → `collect_doc_ids_for_logical_operator` → `filter_doc_ids_by_query`
/// - Check frequency: Every 100 iterations to balance responsiveness vs overhead
/// - Check location: BEFORE expensive operations (regex matching, document loading)
///
/// **ACID Safety**: Only READ operations support deadline (find, count, distinct).
/// WRITE operations (insert, update, delete) pass `None` to preserve atomicity.
#[derive(Debug)]
pub(crate) struct QueryExecutionContext {
    /// Single-field sort optimization info
    pub(crate) sort_field: Option<String>,
    pub(crate) sort_descending: bool,

    /// Whether to defer limit/skip until after sorting
    pub(crate) apply_limit_after_sort: bool,

    /// Pre-fetch parameters (0/None if deferred)
    pub(crate) fetch_skip: usize,
    pub(crate) fetch_limit: Option<usize>,

    /// Original user options for post-processing
    pub(crate) original_skip: usize,
    pub(crate) original_limit: Option<usize>,

    /// Original sort specification
    pub(crate) sort_spec: Option<Vec<(String, i32)>>,

    /// Projection specification
    pub(crate) projection: Option<HashMap<String, i32>>,

    /// Maximum response size in bytes (OOM protection)
    pub(crate) max_response_bytes: Option<usize>,

    /// Cancellation flag for cooperative timeout
    pub(crate) cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,

    /// Deadline for timeout enforcement
    pub(crate) deadline: Option<std::time::Instant>,
}

impl QueryExecutionContext {
    /// Build execution context from FindOptions
    pub(crate) fn from_options(options: &crate::find_options::FindOptions) -> Self {
        let original_skip = options.skip.unwrap_or(0);
        let original_limit = options.limit;

        // Extract single-field sort for index optimization
        let (sort_field, sort_descending) = options
            .sort
            .as_ref()
            .filter(|s| s.len() == 1)
            .map(|s| (Some(s[0].0.clone()), s[0].1 < 0))
            .unwrap_or((None, false));

        // Determine fetch strategy: sort present → load all, then paginate
        // NOTE: We defer limit when sorting because we don't know at this point
        // if the index will be used for sorting. The actual optimization happens
        // in collect_doc_ids_with_options -> try_index_sorted_scan.
        let apply_limit_after_sort = options.sort.is_some();
        let (fetch_skip, fetch_limit) = if apply_limit_after_sort {
            (0, None)
        } else {
            (original_skip, original_limit)
        };

        Self {
            sort_field,
            sort_descending,
            apply_limit_after_sort,
            fetch_skip,
            fetch_limit,
            original_skip,
            original_limit,
            sort_spec: options.sort.clone(),
            projection: options.projection.clone(),
            max_response_bytes: options.max_response_bytes,
            cancel_flag: options.cancel_flag.clone(),
            deadline: options.deadline,
        }
    }

    /// Get sort field reference (for collect_doc_ids_with_options)
    pub(crate) fn sort_field_ref(&self) -> Option<&str> {
        self.sort_field.as_deref()
    }

    /// Determine if in-memory sort is needed (when index didn't sort for us)
    pub(crate) fn needs_memory_sort(&self, index_sorted: bool) -> bool {
        match &self.sort_spec {
            Some(sort) => !(index_sorted && sort.len() == 1),
            None => false,
        }
    }

    /// Apply pagination after sorting (returns owned docs)
    pub(crate) fn apply_post_sort_pagination(&self, docs: Vec<Value>) -> Vec<Value> {
        if self.apply_limit_after_sort {
            crate::find_options::apply_limit_skip(
                docs,
                self.original_limit,
                Some(self.original_skip),
            )
        } else {
            docs
        }
    }

    /// Apply projection to documents (returns owned docs)
    pub(crate) fn apply_projection_to_docs(
        &self,
        docs: Vec<Value>,
    ) -> crate::error::Result<Vec<Value>> {
        match &self.projection {
            Some(proj) => docs
                .into_iter()
                .map(|doc| crate::find_options::apply_projection(&doc, proj))
                .collect(),
            None => Ok(docs),
        }
    }

    /// Check if early projection is safe (before sort)
    ///
    /// Early projection is safe when:
    /// - There is a projection specified AND
    /// - Either no sort is specified OR all sort fields are included in projection
    ///
    /// This allows applying projection right after document load, reducing memory
    /// usage and enabling accurate response size estimation.
    pub(crate) fn can_early_project(&self) -> bool {
        match (&self.projection, &self.sort_spec) {
            (Some(_proj), None) => {
                // No sort - always safe to project early
                true
            }
            (Some(proj), Some(sort)) => {
                // Sort specified - check if all sort fields are in projection
                // Include mode: field must have value 1
                // Exclude mode: field must NOT have value 0
                let is_include_mode = proj.values().any(|&v| v == 1);

                sort.iter().all(|(field, _)| {
                    if is_include_mode {
                        // Include mode: sort field must be explicitly included
                        proj.get(field).map(|&v| v == 1).unwrap_or(false)
                    } else {
                        // Exclude mode: sort field must NOT be excluded
                        proj.get(field).map(|&v| v != 0).unwrap_or(true)
                    }
                })
            }
            (None, _) => {
                // No projection - nothing to apply early
                false
            }
        }
    }

    /// Apply early projection to a single document
    ///
    /// Returns the projected document if early projection is enabled,
    /// otherwise returns the original document unchanged.
    pub(crate) fn apply_early_projection(&self, doc: Value) -> crate::error::Result<Value> {
        match &self.projection {
            Some(proj) => crate::find_options::apply_projection(&doc, proj),
            None => Ok(doc),
        }
    }
}
