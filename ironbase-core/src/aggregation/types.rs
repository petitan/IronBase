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
/// # Default limits (consistent with scale_to_memory(4GB))
///
/// | Limit | Default | Memory Impact |
/// |-------|---------|---------------|
/// | `max_docs_without_match` | 10,000 | ~10 MB (1KB/doc) |
/// | `max_docs_with_match` | 100,000 | ~100 MB |
/// | `max_group_count` | 100,000 | ~6.4 MB (64B/group) |
/// | `max_push_elements` | 10,000/group | ~1 MB/group |
/// | `max_addtoset_elements` | 10,000/group | ~1.5 MB/group |
/// | `max_total_push_elements` | 1,000,000 | ~100 MB global |
/// | `max_total_addtoset_elements` | 500,000 | ~75 MB global |
/// | `max_unwind_output` | 100,000 | ~100 MB |
///
/// # Scaling
///
/// All constructors use 4GB as the reference point (scale_factor = 1.0).
/// Limits scale linearly with available memory up to 2.5× at 10GB+.
///
/// # Example
/// ```rust,ignore
/// use ironbase_core::aggregation::AggregationLimits;
///
/// // Use stricter limits for memory-constrained environment
/// let limits = AggregationLimits {
///     max_docs_without_match: 5_000,
///     max_group_count: 50_000,
///     ..AggregationLimits::low_memory()
/// };
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AggregationLimits {
    /// Maximum documents to scan when there's no $match stage
    /// Default: 10,000 (≈10 MB at 1KB/doc average)
    pub max_docs_without_match: usize,

    /// Maximum documents to scan even WITH $match stage
    /// Prevents OOM when $match returns too many documents
    /// Default: 100,000 (≈100 MB)
    pub max_docs_with_match: usize,

    /// Maximum number of unique groups in $group stage
    /// Streaming $group only uses ~64 bytes per group (hash + accumulator state)
    /// Default: 100,000 (≈6.4 MB)
    pub max_group_count: usize,

    /// Maximum elements in a single $push accumulator PER GROUP
    /// Default: 10,000 per group (≈1 MB/group at 100 bytes/element)
    pub max_push_elements: usize,

    /// Maximum elements in a single $addToSet accumulator PER GROUP
    /// Default: 10,000 per group (≈1.5 MB/group with HashSet overhead)
    pub max_addtoset_elements: usize,

    /// Maximum TOTAL elements across ALL $push accumulators (all groups combined)
    /// Prevents OOM when many groups each have moderate $push usage
    /// Default: 1,000,000 (≈100 MB global)
    pub max_total_push_elements: usize,

    /// Maximum TOTAL elements across ALL $addToSet accumulators (all groups combined)
    /// Prevents OOM when many groups each have moderate $addToSet usage
    /// Default: 500,000 (≈75 MB global)
    pub max_total_addtoset_elements: usize,

    /// Maximum output documents from $unwind stage
    /// Prevents explosion when unwinding large arrays
    /// Default: 100,000 (≈100 MB)
    pub max_unwind_output: usize,

    /// Maximum estimated memory budget in MB
    /// Used for documentation and error messages
    /// Default: 256 MB (25% of 1GB reference)
    pub max_memory_mb: usize,
}

/// Reference memory for scale_factor = 1.0 (4GB)
const BASE_MEMORY_MB: f64 = 4096.0;

impl Default for AggregationLimits {
    /// Default limits consistent with scale_to_memory(4GB)
    ///
    /// FIX (2026-01-25): Harmonized with scale_to_memory() to avoid inconsistencies.
    /// All values now match what scale_to_memory(4GB) would produce.
    fn default() -> Self {
        Self {
            // Document limits
            max_docs_without_match: 10_000, // 10K × 1KB = 10 MB
            max_docs_with_match: 100_000,   // 100K × 1KB = 100 MB

            // Group limits - streaming $group uses ~64 bytes/group
            max_group_count: 100_000, // 100K × 64B = 6.4 MB

            // Per-group accumulator limits
            max_push_elements: 10_000,     // 10K × 100B = 1 MB/group
            max_addtoset_elements: 10_000, // 10K × 150B = 1.5 MB/group

            // Global accumulator limits (across all groups)
            max_total_push_elements: 1_000_000, // 1M × 100B = 100 MB total
            max_total_addtoset_elements: 500_000, // 500K × 150B = 75 MB total

            // Output limits
            max_unwind_output: 100_000, // 100K × 1KB = 100 MB

            // Memory budget (for documentation/error messages)
            max_memory_mb: 256, // 25% of 1GB reference
        }
    }
}

impl AggregationLimits {
    /// Create limits dynamically based on system RAM
    ///
    /// Automatically scales limits to match available system memory:
    /// - Uses max 25% of available RAM for aggregation budget
    /// - Scales all limits proportionally (base = 4GB)
    /// - Falls back to `low_memory()` if detection fails
    ///
    /// # Scaling table (base: 4GB = scale_factor 1.0)
    ///
    /// | Available RAM | scale_factor | max_docs | max_groups | max_memory_mb |
    /// |---------------|--------------|----------|------------|---------------|
    /// | 1 GB          | 0.25         | 2.5K     | 25K        | 64            |
    /// | 2 GB          | 0.5          | 5K       | 50K        | 128           |
    /// | 4 GB          | 1.0          | 10K      | 100K       | 256           |
    /// | 8 GB          | 2.0          | 20K      | 200K       | 512           |
    /// | 10+ GB        | 2.5 (max)    | 25K      | 250K       | 640+          |
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
    ///
    /// FIX (2026-01-25): Unified scaling formula with with_memory_budget()
    /// Both now use BASE_MEMORY_MB (4GB) as reference point.
    fn scale_to_memory(available_bytes: u64) -> Self {
        let available_mb = available_bytes / (1024 * 1024);

        // Use max 25% of available RAM, bounded between 64MB and 4GB
        let max_memory_mb = (available_mb as usize / 4).clamp(64, 4096);

        // Scale factor: 1.0 at 4GB available, proportionally less below, max 2.5 above
        let scale_factor = (available_mb as f64 / BASE_MEMORY_MB).clamp(0.1, 2.5);

        Self::from_scale_factor(scale_factor, max_memory_mb)
    }

    /// Create limits for a specific memory budget
    ///
    /// Scales all limits proportionally to the given memory budget.
    /// Use this when you want precise control over memory usage.
    ///
    /// FIX (2026-01-25): Now uses same 4GB base as scale_to_memory() for consistency.
    ///
    /// # Scaling table (base: 4GB = scale_factor 1.0)
    ///
    /// | Budget     | scale_factor | max_docs | max_groups |
    /// |------------|--------------|----------|------------|
    /// | 256 MB     | 0.0625       | 1K       | 10K        |
    /// | 1 GB       | 0.25         | 2.5K     | 25K        |
    /// | 4 GB       | 1.0          | 10K      | 100K       |
    /// | 8 GB       | 2.0          | 20K      | 200K       |
    /// | 16 GB      | 4.0          | 40K      | 400K       |
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

        // FIX (2026-01-25): Use same 4GB base as scale_to_memory()
        // Old: 1GB base caused 4x inconsistency with scale_to_memory()
        let scale_factor = (effective_budget as f64 / BASE_MEMORY_MB).max(0.01);

        Self::from_scale_factor(scale_factor, effective_budget)
    }

    /// Internal: create limits from scale factor
    ///
    /// Centralizes all limit calculations to ensure consistency.
    fn from_scale_factor(scale_factor: f64, max_memory_mb: usize) -> Self {
        Self {
            // Document limits
            max_docs_without_match: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_docs_with_match: ((100_000.0 * scale_factor) as usize).max(2_000),

            // Group limits - streaming $group uses ~64 bytes/group
            max_group_count: ((100_000.0 * scale_factor) as usize).max(10_000),

            // Per-group accumulator limits
            max_push_elements: ((10_000.0 * scale_factor) as usize).max(1_000),
            max_addtoset_elements: ((10_000.0 * scale_factor) as usize).max(1_000),

            // Global accumulator limits
            max_total_push_elements: ((1_000_000.0 * scale_factor) as usize).max(100_000),
            max_total_addtoset_elements: ((500_000.0 * scale_factor) as usize).max(50_000),

            // Output limits
            max_unwind_output: ((100_000.0 * scale_factor) as usize).max(10_000),

            // Memory budget
            max_memory_mb,
        }
    }

    /// Create limits suitable for low-memory environments (< 1GB RAM)
    ///
    /// FIX (2026-01-25): Increased max_group_count from 500 to 10,000
    /// Streaming $group only uses ~64 bytes/group, so 10K groups = 640KB
    /// The old 500 group limit was unnecessarily restrictive.
    ///
    /// Use this when memory detection fails or in restricted environments.
    pub fn low_memory() -> Self {
        Self {
            // Document limits - very conservative
            max_docs_without_match: 1_000, // 1K × 1KB = 1 MB
            max_docs_with_match: 5_000,    // 5K × 1KB = 5 MB

            // Group limits - 10K groups still only 640KB with streaming
            max_group_count: 10_000, // 10K × 64B = 640 KB

            // Per-group limits
            max_push_elements: 1_000,
            max_addtoset_elements: 1_000,

            // Global limits - reduced for low memory
            max_total_push_elements: 50_000,
            max_total_addtoset_elements: 25_000,

            // Output limits
            max_unwind_output: 10_000,

            // Memory budget
            max_memory_mb: 64,
        }
    }

    /// Create limits with no restrictions (use with caution!)
    ///
    /// WARNING: This can cause OOM on large collections!
    /// Only use for testing or when you have verified the data size is safe.
    pub fn unlimited() -> Self {
        Self {
            max_docs_without_match: usize::MAX,
            max_docs_with_match: usize::MAX,
            max_group_count: usize::MAX,
            max_push_elements: usize::MAX,
            max_addtoset_elements: usize::MAX,
            max_total_push_elements: usize::MAX,
            max_total_addtoset_elements: usize::MAX,
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
    Count(CountStage),
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

/// $count stage - count documents and output as a single field
#[derive(Debug, Clone)]
pub struct CountStage {
    pub(crate) field: String,
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
    /// $substr - substring of a string field
    Substr(StringOperand, usize, usize),
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

/// Operand for string expressions - field reference or literal
#[derive(Debug, Clone)]
pub enum StringOperand {
    Field(String),
    Literal(String),
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
    Substring {
        field: String, // "$field"
        start: usize,
        length: usize,
    },
    /// Nested object: {year: "$year", type: "$doc_type"}
    /// Keys are output field names, values are field references ($ stripped)
    /// Vec preserves key order (MongoDB keeps insertion order), typically 2-5 elements
    Object(Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub enum ValueExpression {
    Field(String),
    Substr {
        field: String, // "$field"
        start: usize,
        length: usize,
    },
    /// Nested object: {title: "$title", year: "$year"}
    /// Recursive: values can be Field, Substr, or nested Object
    Object(Vec<(String, Box<ValueExpression>)>),
}

#[derive(Debug, Clone)]
pub enum Accumulator {
    Sum(SumExpression),
    Avg(String), // Field name
    Min(ValueExpression),
    Max(ValueExpression),
    First(ValueExpression),
    Last(ValueExpression),
    Push(ValueExpression),     // $push - collect all values into array
    AddToSet(ValueExpression), // $addToSet - collect unique values into array
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
