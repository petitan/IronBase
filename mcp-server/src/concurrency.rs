//! Tiered Concurrency Control for IronBase MCP Server
//!
//! Three-tier cost estimation with two-level semaphore system:
//! - Instant: No throttling (indexed point lookups)
//! - Light: Collection semaphore only (bounded indexed queries)
//! - Heavy: Collection + Global semaphore (scans, regex, unbounded)

use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::error::McpError;

/// Maximum concurrent operations per collection (Light + Heavy)
const MAX_CONCURRENT_PER_COLLECTION: usize = 2;

/// Maximum concurrent heavy operations globally (across all collections)
const MAX_CONCURRENT_GLOBAL_HEAVY: usize = 3;

/// Timeout waiting for semaphore acquisition
const SEMAPHORE_TIMEOUT_SECS: u64 = 30;

/// Threshold for "small" limit (Light vs Heavy)
const LIGHT_LIMIT_THRESHOLD: usize = 100;

/// Operation cost classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationCost {
    /// Instant: indexed point lookup, no waiting ever
    Instant,
    /// Light: bounded indexed query, collection semaphore only
    Light,
    /// Heavy: scan/regex/unbounded, both semaphores required
    Heavy,
}

/// Operation type for cost estimation
#[derive(Debug)]
pub enum OperationType<'a> {
    /// find_one query
    FindOne { query: &'a Value },
    /// find query with options
    Find {
        query: &'a Value,
        limit: Option<usize>,
    },
    /// count_documents
    Count { query: &'a Value },
    /// distinct (always heavy)
    Distinct,
    /// aggregate pipeline
    Aggregate { pipeline: &'a [Value] },
    /// fulltext_search
    FulltextSearch { limit: Option<usize> },
    /// fuzzy_search
    FuzzySearch { limit: Option<usize> },
}

/// Concurrency controller with tiered throttling
pub struct ConcurrencyController {
    /// Per-collection semaphores
    collection_semaphores: DashMap<String, Arc<Semaphore>>,
    /// Global heavy operation semaphore
    global_heavy_semaphore: Arc<Semaphore>,
}

impl Default for ConcurrencyController {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrencyController {
    pub fn new() -> Self {
        Self {
            collection_semaphores: DashMap::new(),
            global_heavy_semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_GLOBAL_HEAVY)),
        }
    }

    /// Get or create collection semaphore
    fn collection_semaphore_for(&self, collection: &str) -> Arc<Semaphore> {
        self.collection_semaphores
            .entry(collection.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(MAX_CONCURRENT_PER_COLLECTION)))
            .clone()
    }

    /// Estimate operation cost based on query type and parameters
    pub fn estimate_cost(&self, op: &OperationType) -> OperationCost {
        match op {
            // === INSTANT ===
            // Point lookup by _id - always fast with index
            OperationType::FindOne { query } if Self::is_id_lookup(query) => {
                OperationCost::Instant
            }

            // === LIGHT ===
            // find_one without _id is still light (single doc)
            OperationType::FindOne { .. } => OperationCost::Light,

            // Bounded find without regex
            OperationType::Find {
                query,
                limit: Some(n),
            } if *n <= LIGHT_LIMIT_THRESHOLD && !Self::contains_regex(query) => {
                OperationCost::Light
            }

            // Bounded fulltext search (uses inverted index)
            OperationType::FulltextSearch { limit: Some(n) } if *n <= LIGHT_LIMIT_THRESHOLD => {
                OperationCost::Light
            }

            // Bounded fuzzy search (uses fuzzy index)
            OperationType::FuzzySearch { limit: Some(n) } if *n <= LIGHT_LIMIT_THRESHOLD => {
                OperationCost::Light
            }

            // Count with specific indexed filter (not empty, not regex)
            OperationType::Count { query }
                if !Self::is_empty_query(query) && !Self::contains_regex(query) =>
            {
                OperationCost::Light
            }

            // Aggregate with leading $match (filtered)
            OperationType::Aggregate { pipeline } if Self::has_leading_match(pipeline) => {
                OperationCost::Light
            }

            // === HEAVY ===
            // Everything else: scans, regex, unbounded, distinct
            _ => OperationCost::Heavy,
        }
    }

    /// Execute operation with appropriate throttling
    pub async fn execute<F, R>(
        &self,
        collection: &str,
        cost: OperationCost,
        op: F,
    ) -> crate::error::Result<R>
    where
        F: FnOnce() -> crate::error::Result<R>,
    {
        match cost {
            OperationCost::Instant => {
                tracing::trace!(collection, "Instant op - no throttling");
                op()
            }

            OperationCost::Light => {
                let coll_sem = self.collection_semaphore_for(collection);
                let _permit = Self::acquire_with_timeout(&coll_sem, "collection").await?;
                tracing::debug!(collection, "Light op - collection semaphore acquired");
                op()
            }

            OperationCost::Heavy => {
                // Acquire in consistent order: global first, then collection (prevents deadlock)
                let _global_permit =
                    Self::acquire_with_timeout(&self.global_heavy_semaphore, "global_heavy")
                        .await?;

                let coll_sem = self.collection_semaphore_for(collection);
                let _coll_permit = Self::acquire_with_timeout(&coll_sem, "collection").await?;

                tracing::info!(collection, "Heavy op - both semaphores acquired");
                op()
                // Permits drop here automatically
            }
        }
    }

    /// Synchronous wrapper for use in non-async contexts
    pub fn execute_sync<F, R>(
        &self,
        collection: &str,
        cost: OperationCost,
        op: F,
    ) -> crate::error::Result<R>
    where
        F: FnOnce() -> crate::error::Result<R>,
    {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(self.execute(collection, cost, op)))
        } else {
            // No tokio runtime - execute without throttling (CLI usage, tests)
            op()
        }
    }

    async fn acquire_with_timeout<'a>(
        sem: &'a Semaphore,
        name: &str,
    ) -> crate::error::Result<SemaphorePermit<'a>> {
        tokio::time::timeout(Duration::from_secs(SEMAPHORE_TIMEOUT_SECS), sem.acquire())
            .await
            .map_err(|_| {
                McpError::operation_too_large(format!(
                    "Timeout waiting for {} semaphore ({}s). Server busy with other operations.",
                    name, SEMAPHORE_TIMEOUT_SECS
                ))
            })?
            .map_err(|_| McpError::internal("Semaphore closed unexpectedly"))
    }

    // === Helper functions ===

    /// Check if query is a simple _id lookup
    fn is_id_lookup(query: &Value) -> bool {
        if let Some(obj) = query.as_object() {
            // {"_id": <value>} or {"_id": {"$eq": <value>}}
            if obj.len() == 1 {
                if let Some(id_val) = obj.get("_id") {
                    // Direct value or $eq operator
                    return !id_val.is_object()
                        || id_val
                            .as_object()
                            .map(|o| o.contains_key("$eq"))
                            .unwrap_or(false);
                }
            }
        }
        false
    }

    /// Check if query is empty
    fn is_empty_query(query: &Value) -> bool {
        query.as_object().map(|o| o.is_empty()).unwrap_or(true)
    }

    /// Recursively check for $regex operator
    pub fn contains_regex(value: &Value) -> bool {
        match value {
            Value::Object(map) => {
                for (key, val) in map {
                    if key == "$regex" {
                        return true;
                    }
                    if Self::contains_regex(val) {
                        return true;
                    }
                }
                false
            }
            Value::Array(arr) => arr.iter().any(Self::contains_regex),
            _ => false,
        }
    }

    /// Check if pipeline has leading non-empty $match
    pub fn has_leading_match(pipeline: &[Value]) -> bool {
        if let Some(first) = pipeline.first() {
            if let Some(obj) = first.as_object() {
                if let Some(match_val) = obj.get("$match") {
                    return !Self::is_empty_query(match_val);
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_id_lookup() {
        assert!(ConcurrencyController::is_id_lookup(&json!({"_id": 123})));
        assert!(ConcurrencyController::is_id_lookup(&json!({"_id": "abc"})));
        assert!(ConcurrencyController::is_id_lookup(
            &json!({"_id": {"$eq": 123}})
        ));
        assert!(!ConcurrencyController::is_id_lookup(
            &json!({"name": "test"})
        ));
        assert!(!ConcurrencyController::is_id_lookup(
            &json!({"_id": 1, "name": "test"})
        ));
        assert!(!ConcurrencyController::is_id_lookup(&json!({})));
    }

    #[test]
    fn test_contains_regex() {
        assert!(ConcurrencyController::contains_regex(
            &json!({"name": {"$regex": "^test"}})
        ));
        assert!(ConcurrencyController::contains_regex(
            &json!({"$or": [{"name": {"$regex": "x"}}]})
        ));
        assert!(ConcurrencyController::contains_regex(
            &json!({"$and": [{"a": 1}, {"b": {"$regex": "y"}}]})
        ));
        assert!(!ConcurrencyController::contains_regex(
            &json!({"name": "test"})
        ));
        assert!(!ConcurrencyController::contains_regex(&json!({})));
    }

    #[test]
    fn test_has_leading_match() {
        assert!(ConcurrencyController::has_leading_match(&[json!({"$match": {"status": "active"}})]));
        assert!(ConcurrencyController::has_leading_match(&[
            json!({"$match": {"x": 1}}),
            json!({"$group": {"_id": "$a"}})
        ]));
        assert!(!ConcurrencyController::has_leading_match(&[json!({"$group": {"_id": "$a"}})]));
        assert!(!ConcurrencyController::has_leading_match(&[json!({"$match": {}})]));
        assert!(!ConcurrencyController::has_leading_match(&[]));
    }

    #[test]
    fn test_cost_estimation_instant() {
        let ctrl = ConcurrencyController::new();

        // _id lookup is instant
        let op = OperationType::FindOne {
            query: &json!({"_id": 1}),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Instant);

        let op = OperationType::FindOne {
            query: &json!({"_id": {"$eq": "abc"}}),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Instant);
    }

    #[test]
    fn test_cost_estimation_light() {
        let ctrl = ConcurrencyController::new();

        // find_one without _id is light
        let op = OperationType::FindOne {
            query: &json!({"email": "test@example.com"}),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);

        // Bounded find without regex
        let op = OperationType::Find {
            query: &json!({"status": "active"}),
            limit: Some(50),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);

        // Bounded fulltext search
        let op = OperationType::FulltextSearch { limit: Some(10) };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);

        // Bounded fuzzy search
        let op = OperationType::FuzzySearch { limit: Some(100) };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);

        // Count with non-empty query
        let op = OperationType::Count {
            query: &json!({"status": "active"}),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);

        // Aggregate with $match
        let op = OperationType::Aggregate {
            pipeline: &[json!({"$match": {"x": 1}}), json!({"$group": {"_id": "$a"}})],
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Light);
    }

    #[test]
    fn test_cost_estimation_heavy() {
        let ctrl = ConcurrencyController::new();

        // Find with regex (even with small limit)
        let op = OperationType::Find {
            query: &json!({"name": {"$regex": "^test"}}),
            limit: Some(10),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Unbounded find
        let op = OperationType::Find {
            query: &json!({}),
            limit: None,
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Large limit find
        let op = OperationType::Find {
            query: &json!({"status": "active"}),
            limit: Some(1000),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Distinct is always heavy
        let op = OperationType::Distinct;
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Empty count
        let op = OperationType::Count { query: &json!({}) };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Count with regex
        let op = OperationType::Count {
            query: &json!({"name": {"$regex": "x"}}),
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Aggregate without $match
        let op = OperationType::Aggregate {
            pipeline: &[json!({"$group": {"_id": "$a"}})],
        };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Unbounded fulltext
        let op = OperationType::FulltextSearch { limit: None };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);

        // Large limit fulltext
        let op = OperationType::FulltextSearch { limit: Some(500) };
        assert_eq!(ctrl.estimate_cost(&op), OperationCost::Heavy);
    }
}
