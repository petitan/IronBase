// src/aggregation/types.rs
// Type definitions for aggregation pipeline

use crate::query::Query;
use serde_json::Value;
use std::collections::HashMap;

/// Aggregation pipeline
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub(crate) stages: Vec<Stage>,
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
