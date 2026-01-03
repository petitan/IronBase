// src/aggregation/types.rs
// Type definitions for aggregation pipeline

use crate::query::Query;
use serde_json::Value;
use std::collections::HashMap;

/// Aggregation limits to prevent OOM on large collections
///
/// These limits protect against memory exhaustion when running aggregation
/// pipelines without $match (full collection scans) or with high-cardinality $group.
///
/// # Default limits
/// - `max_docs_without_match`: 100,000 - Max documents to process without $match
/// - `max_group_count`: 50,000 - Max unique groups in $group stage
/// - `max_memory_mb`: 512 - Max estimated memory usage (MB)
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::aggregation::AggregationLimits;
///
/// // Use stricter limits for memory-constrained environment
/// let limits = AggregationLimits {
///     max_docs_without_match: 10_000,
///     max_group_count: 5_000,
///     max_memory_mb: 256,
/// };
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AggregationLimits {
    /// Maximum documents to scan when there's no $match stage
    /// Default: 100,000
    pub max_docs_without_match: usize,

    /// Maximum documents to scan even WITH $match stage
    /// Prevents OOM when $match returns too many documents
    /// Default: 1,000,000 (was: usize::MAX - UNSAFE!)
    pub max_docs_with_match: usize,

    /// Maximum number of unique groups in $group stage
    /// Prevents memory explosion with high-cardinality group keys
    /// Default: 50,000
    pub max_group_count: usize,

    /// Maximum elements in a single $push accumulator per group
    /// Prevents OOM when collecting many values
    /// Default: 100,000
    pub max_push_elements: usize,

    /// Maximum elements in a single $addToSet accumulator per group
    /// Prevents OOM with high-cardinality fields
    /// Default: 100,000
    pub max_addtoset_elements: usize,

    /// Maximum output documents from $unwind stage
    /// Prevents explosion when unwinding large arrays
    /// Default: 1,000,000
    pub max_unwind_output: usize,

    /// Maximum estimated memory usage in MB
    /// NOTE: Currently used for try_reserve() failures, not runtime tracking
    /// Default: 512 MB
    pub max_memory_mb: usize,
}

impl Default for AggregationLimits {
    fn default() -> Self {
        Self {
            // OOM FIX (2026-01): Reduced from 100K to 10K
            // 10K docs × 100KB avg = ~1GB max memory - safe for most systems
            // Use aggregate_auto() or explicit $match for larger collections
            max_docs_without_match: 10_000,
            max_docs_with_match: 1_000_000,
            max_group_count: 50_000,
            max_push_elements: 100_000,
            max_addtoset_elements: 100_000,
            max_unwind_output: 1_000_000,
            max_memory_mb: 512,
        }
    }
}

impl AggregationLimits {
    /// Create limits dynamically based on system RAM
    ///
    /// Automatically scales limits to match available system memory:
    /// - Uses max 25% of available RAM for aggregation
    /// - Scales doc/group limits proportionally
    /// - Falls back to `low_memory()` if detection fails
    ///
    /// # Scaling table (OOM FIX 2026-01: reduced base from 100K to 10K)
    /// | Available RAM | max_memory_mb | max_docs_without_match | max_groups |
    /// |---------------|---------------|------------------------|------------|
    /// | < 512 MB      | 64            | 1K                     | 500        |
    /// | 512MB - 2GB   | 128           | 5K                     | 2.5K       |
    /// | 2GB - 8GB     | 256           | 10K                    | 5K         |
    /// | 8GB - 32GB    | 512           | 25K                    | 10K        |
    /// | > 32GB        | 1024          | 50K                    | 25K        |
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::aggregation::AggregationLimits;
    ///
    /// let limits = AggregationLimits::from_system_memory();
    /// collection.aggregate_with_limits(&pipeline, limits)?;
    /// ```
    pub fn from_system_memory() -> Self {
        use super::memory_info::get_available_memory_bytes;

        match get_available_memory_bytes() {
            Some(bytes) => Self::scale_to_memory(bytes),
            None => {
                // Detection failed - use safe defaults
                Self::low_memory()
            }
        }
    }

    /// Scale limits based on available memory bytes
    fn scale_to_memory(available_bytes: u64) -> Self {
        let available_mb = available_bytes / (1024 * 1024);

        // Use max 25% of available RAM, bounded between 64MB and 4GB
        let max_memory_mb = (available_mb as usize / 4).clamp(64, 4096);

        // Scale factor: 1.0 at 4GB available, proportionally less below
        // This means at 4GB+ available, you get the "standard" limits
        let scale_factor = (available_mb as f64 / 4096.0).clamp(0.1, 2.5);

        Self {
            // Document limits scale with memory
            // OOM FIX (2026-01): Reduced base from 100K to 10K
            // Even with 8GB RAM, 100K docs × 100KB = 10GB > RAM!
            max_docs_without_match: ((10_000.0 * scale_factor) as usize).max(1_000),
            // OOM FIX (2026-01): Reduced minimum from 10K to 2K
            // $match doesn't disable limits - it just allows higher ones
            max_docs_with_match: ((100_000.0 * scale_factor) as usize).max(2_000),

            // Group/accumulator limits (also reduced proportionally)
            max_group_count: ((5_000.0 * scale_factor) as usize).max(500),
            max_push_elements: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_addtoset_elements: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_unwind_output: ((100_000.0 * scale_factor) as usize).max(10_000),

            // Memory limit
            max_memory_mb,
        }
    }

    /// Create limits for a specific memory budget
    ///
    /// Scales all limits proportionally to the given memory budget.
    /// Use this when you want precise control over memory usage.
    ///
    /// # Scaling table
    /// | Budget     | max_docs_without_match | max_groups |
    /// |------------|------------------------|------------|
    /// | 64 MB      | 1K (minimum)           | 500        |
    /// | 256 MB     | 2.5K                   | 1.25K      |
    /// | 1 GB       | 10K                    | 5K         |
    /// | 4 GB       | 40K                    | 20K        |
    /// | 16 GB      | 160K                   | 80K        |
    ///
    /// # Arguments
    /// * `memory_mb` - Maximum memory to use in megabytes (min: 64, no upper limit)
    ///
    /// # Example
    /// ```rust,ignore
    /// use ironbase_core::aggregation::AggregationLimits;
    ///
    /// // Limit aggregation to 256 MB
    /// let limits = AggregationLimits::with_memory_budget(256);
    /// collection.aggregate_with_limits(&pipeline, limits)?;
    ///
    /// // Large memory budget (16 GB) for heavy aggregations
    /// let limits = AggregationLimits::with_memory_budget(16384);
    /// ```
    pub fn with_memory_budget(memory_mb: usize) -> Self {
        // Minimum 64MB, no upper limit (allow large memory systems to scale)
        let effective_budget = memory_mb.max(64);

        // Scale factor: 1.0 at 1GB budget, proportionally more/less for other values
        // This means at 1GB budget, you get the "base" limits (10K docs, 5K groups)
        // At 4GB → 4x limits, at 16GB → 16x limits
        let scale_factor = (effective_budget as f64 / 1024.0).max(0.1);

        Self {
            max_docs_without_match: ((10_000.0 * scale_factor) as usize).max(1_000),
            // OOM FIX (2026-01): Reduced minimum from 10K to 2K
            max_docs_with_match: ((100_000.0 * scale_factor) as usize).max(2_000),
            max_group_count: ((5_000.0 * scale_factor) as usize).max(500),
            max_push_elements: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_addtoset_elements: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_unwind_output: ((100_000.0 * scale_factor) as usize).max(10_000),
            max_memory_mb: effective_budget,
        }
    }

    /// Create limits suitable for low-memory environments
    ///
    /// OOM FIX (2026-01): Made truly conservative
    /// - 1K docs without match (was: 10K - same as default!)
    /// - 5K docs with match (was: 100K)
    /// - 64 MB memory budget
    ///
    /// Use this when memory detection fails or in restricted environments.
    pub fn low_memory() -> Self {
        Self {
            max_docs_without_match: 1_000, // Conservative - 1K × 100KB = 100MB max
            max_docs_with_match: 5_000,    // Even with $match, limit aggressively
            max_group_count: 500,          // Low group count
            max_push_elements: 1_000,
            max_addtoset_elements: 1_000,
            max_unwind_output: 10_000,
            max_memory_mb: 128,
        }
    }

    /// Create limits with no restrictions (use with caution!)
    /// WARNING: This can cause OOM on large collections!
    pub fn unlimited() -> Self {
        Self {
            max_docs_without_match: usize::MAX,
            max_docs_with_match: usize::MAX,
            max_group_count: usize::MAX,
            max_push_elements: usize::MAX,
            max_addtoset_elements: usize::MAX,
            max_unwind_output: usize::MAX,
            max_memory_mb: usize::MAX,
        }
    }
}

/// Aggregation pipeline
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub(crate) stages: Vec<Stage>,
    /// Whether this pipeline has a leading $match (affects limits)
    pub(crate) has_leading_match: bool,
}

impl Pipeline {
    /// Extract the leading $match stage query for index optimization
    ///
    /// If the first stage is $match, returns its query as JSON and removes it from the pipeline.
    /// This allows the caller to use an indexed find() instead of a full collection scan.
    ///
    /// Returns None if the first stage is not $match.
    pub fn extract_leading_match(&mut self) -> Option<Value> {
        if let Some(Stage::Match(_)) = self.stages.first() {
            // Remove and take ownership of the first stage
            if let Stage::Match(match_stage) = self.stages.remove(0) {
                return Some(match_stage.query.into_json());
            }
        }
        None
    }

    /// Get leading $group stage reference for index optimization check
    ///
    /// Returns reference to the $group stage if it's the first stage.
    /// Used to check if index-based group optimization is possible.
    pub fn peek_leading_group(&self) -> Option<&GroupStage> {
        if let Some(Stage::Group(group_stage)) = self.stages.first() {
            Some(group_stage)
        } else {
            None
        }
    }

    /// Remove the leading $group stage (after index-based execution)
    ///
    /// Call this after successfully executing $group using index optimization.
    /// The remaining stages will be executed on the indexed results.
    pub fn remove_leading_group(&mut self) {
        if let Some(Stage::Group(_)) = self.stages.first() {
            self.stages.remove(0);
        }
    }
}

/// Pipeline stage
#[derive(Debug, Clone)]
pub enum Stage {
    Match(MatchStage),
    Project(ProjectStage),
    Group(GroupStage),
    Sort(SortStage),
    Limit(LimitStage),
    Skip(SkipStage),
    Unwind(UnwindStage),
}

/// $match stage - filter documents
#[derive(Debug, Clone)]
pub struct MatchStage {
    pub(crate) query: Query,
}

/// $project stage - reshape documents
#[derive(Debug, Clone)]
pub struct ProjectStage {
    pub(crate) fields: HashMap<String, ProjectField>,
}

#[derive(Debug, Clone)]
pub enum ProjectField {
    Include,                       // 1
    Exclude,                       // 0
    Rename(String),                // "$fieldName"
    Expression(ProjectExpression), // {"$size": "$field"}, etc.
}

/// Expressions that can be used in $project stage
#[derive(Debug, Clone)]
pub enum ProjectExpression {
    /// $size - returns the length of an array field
    Size(String), // Field name (e.g., "$tags" -> "tags")
    /// $reduce - apply a custom reduction to an array
    Reduce(ReduceExpression),
    /// $add - add numbers: { $add: ["$price", "$tax"] }
    Add(Vec<ArithmeticOperand>),
    /// $subtract - subtract: { $subtract: ["$price", "$discount"] }
    Subtract(Box<ArithmeticOperand>, Box<ArithmeticOperand>),
    /// $multiply - multiply: { $multiply: ["$price", "$qty"] }
    Multiply(Vec<ArithmeticOperand>),
    /// $divide - divide: { $divide: ["$total", "$count"] }
    Divide(Box<ArithmeticOperand>, Box<ArithmeticOperand>),
    /// $mod - modulo: { $mod: ["$value", 10] }
    Mod(Box<ArithmeticOperand>, Box<ArithmeticOperand>),
    /// $abs - absolute value: { $abs: "$diff" }
    Abs(Box<ArithmeticOperand>),
    /// $ceil - ceiling: { $ceil: "$value" }
    Ceil(Box<ArithmeticOperand>),
    /// $floor - floor: { $floor: "$value" }
    Floor(Box<ArithmeticOperand>),
    /// $round - round: { $round: ["$value", 2] }
    Round(Box<ArithmeticOperand>, Option<i32>),
}

/// Operand for arithmetic expressions - can be field reference, literal, or nested expression
#[derive(Debug, Clone)]
pub enum ArithmeticOperand {
    /// Field reference: "$price"
    Field(String),
    /// Literal number
    Literal(f64),
    /// Nested expression: { $add: [...] }
    Expression(Box<ProjectExpression>),
}

/// $reduce expression - reduces an array to a single value
///
/// # MongoDB Syntax
///
/// ```json
/// {$reduce: {
///     input: "$arrayField",
///     initialValue: 0,
///     in: {$add: ["$$value", "$$this"]}
/// }}
/// ```
///
/// Special variables:
/// - `$$value` - the accumulated value from previous iterations
/// - `$$this` - the current array element
#[derive(Debug, Clone)]
pub struct ReduceExpression {
    /// Input array field name (without $)
    pub(crate) input: String,
    /// Initial value for the accumulator
    pub(crate) initial_value: Value,
    /// Reduction expression to apply
    pub(crate) in_expr: ReduceInExpr,
}

/// Supported reduction operations
#[derive(Debug, Clone)]
pub enum ReduceInExpr {
    /// {$add: ["$$value", "$$this"]} - sum values
    Add,
    /// {$add: ["$$value", "$$this.field"]} - sum field values from objects
    AddField(String),
    /// {$multiply: ["$$value", "$$this"]} - multiply values
    Multiply,
    /// {$multiply: ["$$value", "$$this.field"]} - multiply field values from objects
    MultiplyField(String),
    /// {$concat: ["$$value", "$$this"]} - concatenate strings
    Concat,
    /// {$concat: ["$$value", "$$this.field"]} - concatenate field values from objects
    ConcatField(String),
    /// {$concat: ["$$value", separator, "$$this"]} - concatenate with separator
    ConcatWithSeparator(String),
    /// {$concat: ["$$value", separator, "$$this.field"]} - concat fields with separator
    ConcatFieldWithSeparator { field: String, separator: String },
}

/// $group stage - group documents and compute aggregates
#[derive(Debug, Clone)]
pub struct GroupStage {
    pub(crate) id: GroupId,
    pub(crate) accumulators: HashMap<String, Accumulator>,
}

#[derive(Debug, Clone)]
pub enum GroupId {
    Field(String), // "$city"
    Null,          // null (all documents in one group)
}

#[derive(Debug, Clone)]
pub enum Accumulator {
    Sum(SumExpression),
    Avg(String), // Field name
    Min(String),
    Max(String),
    First(String),
    Last(String),
    Push(String),     // $push - collect all values into array
    AddToSet(String), // $addToSet - collect unique values into array
}

/// Streaming accumulator state - stores only the accumulated value, NOT full documents
/// This reduces memory from O(N * doc_size) to O(G * state_size) where G = number of groups
#[derive(Debug, Clone)]
pub enum AccumulatorState {
    /// Sum state: tracks integer and float sums separately for precision
    Sum {
        int_sum: i64,
        float_sum: f64,
        has_float: bool,
        is_count: bool, // true if $sum: 1 (counting)
    },
    /// Avg state: tracks sum and count
    Avg { sum: f64, count: usize },
    /// Min state: tracks minimum value seen
    Min { value: Option<Value> },
    /// Max state: tracks maximum value seen
    Max { value: Option<Value> },
    /// First state: captures first value only
    /// `captured` tracks if we've processed the first doc (even if field was missing)
    First {
        value: Option<Value>,
        captured: bool,
    },
    /// Last state: always updates to latest value
    /// We need to track the actual last doc's value, even if it was missing/null
    Last {
        value: Option<Value>,
        doc_count: usize,
    },
    /// Push state: must store all values (no optimization possible)
    Push { values: Vec<Value> },
    /// AddToSet state: stores unique values
    AddToSet {
        seen: std::collections::HashSet<String>,
        values: Vec<Value>,
    },
}

#[derive(Debug, Clone)]
pub enum SumExpression {
    Constant(i64), // {"$sum": 1} - count
    Field(String), // {"$sum": "$amount"} - sum field values
}

/// $sort stage - sort documents
#[derive(Debug, Clone)]
pub struct SortStage {
    pub(crate) fields: Vec<(String, SortDirection)>,
}

#[derive(Debug, Clone)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// $limit stage - limit number of documents
#[derive(Debug, Clone)]
pub struct LimitStage {
    pub(crate) limit: usize,
}

/// $skip stage - skip documents
#[derive(Debug, Clone)]
pub struct SkipStage {
    pub(crate) skip: usize,
}

/// $unwind stage - deconstruct an array field
///
/// Outputs one document per array element. The path field in each output document
/// is replaced with the array element value.
///
/// # MongoDB Syntax
///
/// Simple form: `{$unwind: "$arrayField"}`
///
/// Extended form:
/// ```json
/// {$unwind: {
///     path: "$arrayField",
///     includeArrayIndex: "indexField",      // optional
///     preserveNullAndEmptyArrays: true      // optional
/// }}
/// ```
#[derive(Debug, Clone)]
pub struct UnwindStage {
    /// Field path to unwind (without leading $)
    pub(crate) path: String,
    /// Optional field name to store array index
    pub(crate) include_array_index: Option<String>,
    /// If true, preserve documents with null/missing/empty arrays
    pub(crate) preserve_null_and_empty_arrays: bool,
}
