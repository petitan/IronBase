// src/aggregation/stages/match_stage.rs
// $match stage implementation

use crate::aggregation::types::MatchStage;
use crate::document::{Document, DocumentId};
use crate::error::Result;
use crate::query::Query;
use serde_json::Value;
use std::collections::HashMap;

/// Dynamic threshold for parallel processing based on available CPU cores.
/// Returns `usize::MAX` on single-core (rayon overhead > benefit).
/// On multi-core: `cpus * 100` (4 cores → 400, 8 → 800, 16 → 1600).
#[cfg(feature = "parallel")]
fn parallel_threshold() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    if cpus <= 1 {
        usize::MAX
    } else {
        cpus * 100
    }
}

impl MatchStage {
    pub(crate) fn from_json(spec: &Value) -> Result<Self> {
        let query = Query::from_json(spec)?;
        Ok(MatchStage { query })
    }

    /// Check if a single document matches this stage's query
    ///
    /// Used for streaming execution where we filter documents one at a time.
    pub(crate) fn matches(&self, doc: &Value) -> Result<bool> {
        // An intermediate result's `_id` can be any value ($group: object,
        // null, float) or absent ($project). The matcher reads the fields, so
        // build the Document from them as they are; `Document::from_value`
        // would require an int/string `_id` (audit 2026-10-07 A1)
        let Value::Object(obj) = doc else {
            return Ok(false);
        };
        let fields: HashMap<String, Value> =
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let document = Document::new(DocumentId::Int(0), fields);

        self.query.matches(&document)
    }

    /// Execute $match on a batch of documents (Phase 3: post-materialization path)
    ///
    /// When `parallel` feature is enabled and the batch is large enough,
    /// uses rayon `into_par_iter()` for CPU-bound query matching across multiple cores.
    /// The threshold is dynamic: `available_cpus * 100` (1 core → never parallelize).
    ///
    /// The streaming path (pipeline leading $match) is NOT affected — it remains
    /// iterator-based in `apply_streamable_stages()` / `MatchIterator`.
    pub(crate) fn execute(&self, docs: Vec<Value>) -> Result<Vec<Value>> {
        #[cfg(feature = "parallel")]
        {
            let threshold = parallel_threshold();
            if docs.len() > threshold {
                use rayon::prelude::*;

                let results: Vec<Value> = docs
                    .into_par_iter()
                    .filter(|doc| self.matches(doc).unwrap_or(false))
                    .collect();
                return Ok(results);
            }
        }

        // Sequential path: small batches, single core, or parallel feature disabled
        let mut results = Vec::new();

        for doc in docs {
            if self.matches(&doc)? {
                results.push(doc);
            }
        }

        Ok(results)
    }
}
