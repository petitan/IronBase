// Index key types and ordering

use serde::{Deserialize, Serialize};

/// Index key - supported types for indexing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(OrderedFloat),
    String(String),
    /// Compound key for multi-field indexes (e.g., ["country", "city"])
    Compound(Vec<IndexKey>),
    /// Sentinel value for "greater than everything" - used for range scan upper bounds
    MaxKey,
}

/// OrderedFloat wrapper for f64 to enable Ord
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OrderedFloat(pub f64);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.0.is_nan(), other.0.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => self
                .0
                .partial_cmp(&other.0)
                .unwrap_or(std::cmp::Ordering::Equal),
        }
    }
}

/// Implement Ord for IndexKey - defines ordering for B+ tree
impl PartialOrd for IndexKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IndexKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use IndexKey::*;
        // Ordering: Null < Bool < Int < Float < String < Compound < MaxKey
        match (self, other) {
            // MaxKey is greater than everything (except itself)
            (MaxKey, MaxKey) => std::cmp::Ordering::Equal,
            (MaxKey, _) => std::cmp::Ordering::Greater,
            (_, MaxKey) => std::cmp::Ordering::Less,

            (Null, Null) => std::cmp::Ordering::Equal,
            (Null, _) => std::cmp::Ordering::Less,
            (_, Null) => std::cmp::Ordering::Greater,

            (Bool(a), Bool(b)) => a.cmp(b),
            (Bool(_), _) => std::cmp::Ordering::Less,
            (_, Bool(_)) => std::cmp::Ordering::Greater,

            (Int(a), Int(b)) => a.cmp(b),
            (Int(_), _) => std::cmp::Ordering::Less,
            (_, Int(_)) => std::cmp::Ordering::Greater,

            (Float(a), Float(b)) => a.cmp(b),
            (Float(_), _) => std::cmp::Ordering::Less,
            (_, Float(_)) => std::cmp::Ordering::Greater,

            (String(a), String(b)) => a.cmp(b),
            (String(_), Compound(_)) => std::cmp::Ordering::Less,

            // Compound keys - compare element by element (lexicographic order)
            (Compound(a), Compound(b)) => a.cmp(b),
            (Compound(_), _) => std::cmp::Ordering::Greater,
        }
    }
}

/// Convert serde_json::Value reference to IndexKey (borrows, must clone strings)
impl From<&serde_json::Value> for IndexKey {
    fn from(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => IndexKey::Null,
            serde_json::Value::Bool(b) => IndexKey::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    IndexKey::Int(i)
                } else if let Some(f) = n.as_f64() {
                    IndexKey::Float(OrderedFloat(f))
                } else {
                    IndexKey::Null
                }
            }
            serde_json::Value::String(s) => IndexKey::String(s.clone()),
            _ => IndexKey::Null, // Arrays and objects -> Null for simple index
        }
    }
}

/// Convert owned serde_json::Value to IndexKey (takes ownership, zero-copy for strings)
impl From<serde_json::Value> for IndexKey {
    fn from(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => IndexKey::Null,
            serde_json::Value::Bool(b) => IndexKey::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    IndexKey::Int(i)
                } else if let Some(f) = n.as_f64() {
                    IndexKey::Float(OrderedFloat(f))
                } else {
                    IndexKey::Null
                }
            }
            serde_json::Value::String(s) => IndexKey::String(s), // Zero-copy: takes ownership
            _ => IndexKey::Null, // Arrays and objects -> Null for simple index
        }
    }
}

/// Index prefix information for QueryPlanner (compound index aware)
#[derive(Debug, Clone)]
pub struct IndexPrefixInfo {
    /// Index name
    pub index_name: String,
    /// First (prefix) field name - used for matching queries
    pub prefix_field: String,
    /// Whether this is a compound index
    pub is_compound: bool,
    /// Total number of fields in the index
    pub num_fields: usize,
}
