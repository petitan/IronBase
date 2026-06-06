// Index key types and ordering

use super::btree::{Histogram, MostCommonValues};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// Index key - supported types for indexing
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

impl Hash for OrderedFloat {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the bits of the float to ensure consistency with Eq
        self.0.to_bits().hash(state);
    }
}

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

/// Next Unicode scalar value after `c`, skipping the UTF-16 surrogate gap.
/// Returns `None` when `c == char::MAX` (no successor exists).
fn next_scalar(c: char) -> Option<char> {
    let mut cp = c as u32 + 1;
    if cp == 0xD800 {
        cp = 0xE000; // 0xD800..=0xDFFF are surrogates, not valid scalar values
    }
    char::from_u32(cp)
}

/// Exclusive upper bound for a "starts-with `prefix`" range scan: the smallest
/// key strictly greater than every string that starts with `prefix`. Use it
/// with a half-open `[prefix, upper)` range (inclusive start, exclusive end).
///
/// The earlier bound `prefix + '\u{10ffff}'` combined with an *inclusive* end
/// silently dropped any stored value of the form `prefix + '\u{10ffff}' + …`,
/// because such a value sorts strictly above `prefix + '\u{10ffff}'` (audit
/// 2026-06-06 finding H). This computes the true successor by incrementing the
/// last scalar of `prefix` (skipping the surrogate gap), carrying when it is
/// already `char::MAX`. Returns [`IndexKey::MaxKey`] (scan to the end of the
/// key space) when no finite successor exists — an empty prefix, or one made
/// up solely of `char::MAX`.
pub(crate) fn prefix_scan_upper_bound(prefix: &str) -> IndexKey {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = next_scalar(last) {
            let mut s: String = chars.into_iter().collect();
            s.push(next);
            return IndexKey::String(s);
        }
        // `last` was char::MAX — drop it and carry into the previous scalar.
    }
    IndexKey::MaxKey
}

/// A numeric query interval split onto the two disjoint key buckets the B+ tree
/// stores numbers in. `IndexKey` orders ALL `Int` below ALL `Float`, so a single
/// range like `[Int(18), Int(65)]` cannot contain any `Float` key — yet the
/// range operators compare numerically and `compare_values` only ever compares
/// numbers to numbers (cross-type → `None` → no match). Scanning/counting BOTH
/// buckets therefore matches operator semantics exactly (audit 2026-06-06
/// finding D). One bucket may be empty.
#[derive(Debug, Clone)]
pub(crate) struct NumericBucketRanges {
    /// `(start, end, inclusive_start, inclusive_end)` for the Int bucket, or
    /// `None` when no integer falls inside the interval.
    pub int_range: Option<(IndexKey, IndexKey, bool, bool)>,
    /// `(start, end, inclusive_start, inclusive_end)` for the Float bucket.
    pub float_range: (IndexKey, IndexKey, bool, bool),
}

fn index_key_as_f64(k: &IndexKey) -> Option<f64> {
    match k {
        IndexKey::Int(n) => Some(*n as f64),
        IndexKey::Float(f) => Some(f.0),
        _ => None,
    }
}

/// Split an `IndexRangeScan`'s bounds into Int + Float buckets when they describe
/// a NUMERIC interval (each present bound is `Int`/`Float`, and at least one bound
/// is present). Returns `None` for non-numeric ranges (e.g. string ranges), which
/// must use the ordinary single-range scan. See [`NumericBucketRanges`].
pub(crate) fn numeric_range_buckets(
    start: Option<&IndexKey>,
    end: Option<&IndexKey>,
    inclusive_start: bool,
    inclusive_end: bool,
) -> Option<NumericBucketRanges> {
    // Any present bound that is not numeric disqualifies this as a numeric range.
    if let Some(s) = start {
        index_key_as_f64(s)?;
    }
    if let Some(e) = end {
        index_key_as_f64(e)?;
    }
    let lo = start.and_then(index_key_as_f64);
    let hi = end.and_then(index_key_as_f64);
    // Require at least one numeric bound (a fully unbounded range is not numeric).
    if lo.is_none() && hi.is_none() {
        return None;
    }

    // Float bucket: the same numeric interval expressed in float key space.
    let float_start = IndexKey::Float(OrderedFloat(lo.unwrap_or(f64::NEG_INFINITY)));
    let float_end = IndexKey::Float(OrderedFloat(hi.unwrap_or(f64::INFINITY)));
    let float_incl_start = lo.is_none() || inclusive_start;
    let float_incl_end = hi.is_none() || inclusive_end;

    // Int bucket: integers inside the interval, derived EXACTLY from the original
    // keys. An Int bound is used verbatim — NOT round-tripped through f64, which
    // mis-rounded integer bounds beyond ±2^53 and saturated bounds beyond the i64
    // range to a spurious boundary key (both made the no-post-filter count fast
    // path diverge from find). A Float bound is rounded toward the interval
    // interior with i64-range detection: an out-of-range Float collapses the Int
    // bucket to empty so only the Float bucket carries the (correct) match.
    let int_lo = int_bound_low(start, inclusive_start);
    let int_hi = int_bound_high(end, inclusive_end);
    let int_range = match (int_lo, int_hi) {
        (Some(lo_i), Some(hi_i)) if lo_i <= hi_i => {
            Some((IndexKey::Int(lo_i), IndexKey::Int(hi_i), true, true))
        }
        _ => None,
    };

    Some(NumericBucketRanges {
        int_range,
        float_range: (float_start, float_end, float_incl_start, float_incl_end),
    })
}

// i64 range endpoints as f64 — note `i64::MAX as f64` rounds UP to 2^63, which is
// why the low-bound check below uses `>=` (no i64 is >= 2^63).
const I64_MIN_F: f64 = i64::MIN as f64;
const I64_MAX_F: f64 = i64::MAX as f64;

/// Smallest i64 satisfying the lower bound (`>= v` if inclusive, `> v` if not),
/// or `None` if no i64 satisfies it (bound above the i64 range / overflow).
fn int_bound_low(start: Option<&IndexKey>, inclusive: bool) -> Option<i64> {
    match start {
        None => Some(i64::MIN),
        Some(IndexKey::Int(n)) => {
            if inclusive {
                Some(*n)
            } else {
                n.checked_add(1) // n > i64::MAX → no i64 qualifies
            }
        }
        Some(IndexKey::Float(f)) => {
            // smallest integer >= v (incl) or > v (excl)
            let b = if inclusive {
                f.0.ceil()
            } else {
                f.0.floor() + 1.0
            };
            if b >= I64_MAX_F {
                None
            } else if b < I64_MIN_F {
                Some(i64::MIN)
            } else {
                Some(b as i64)
            }
        }
        _ => None,
    }
}

/// Largest i64 satisfying the upper bound (`<= v` if inclusive, `< v` if not),
/// or `None` if no i64 satisfies it (bound below the i64 range / overflow).
fn int_bound_high(end: Option<&IndexKey>, inclusive: bool) -> Option<i64> {
    match end {
        None => Some(i64::MAX),
        Some(IndexKey::Int(n)) => {
            if inclusive {
                Some(*n)
            } else {
                n.checked_sub(1) // n < i64::MIN → no i64 qualifies
            }
        }
        Some(IndexKey::Float(f)) => {
            // largest integer <= v (incl) or < v (excl)
            let b = if inclusive {
                f.0.floor()
            } else {
                f.0.ceil() - 1.0
            };
            if b < I64_MIN_F {
                None
            } else if b > I64_MAX_F {
                Some(i64::MAX)
            } else {
                Some(b as i64)
            }
        }
        _ => None,
    }
}

impl IndexKey {
    /// Convert IndexKey back to serde_json::Value
    ///
    /// Used for index-based distinct() to extract unique values without loading documents.
    pub fn to_value(&self) -> serde_json::Value {
        match self {
            IndexKey::Null => serde_json::Value::Null,
            IndexKey::Bool(b) => serde_json::Value::Bool(*b),
            IndexKey::Int(i) => serde_json::Value::Number((*i).into()),
            IndexKey::Float(f) => serde_json::Number::from_f64(f.0)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            IndexKey::String(s) => serde_json::Value::String(s.clone()),
            IndexKey::Compound(keys) => {
                // Convert compound keys to array
                serde_json::Value::Array(keys.iter().map(|k| k.to_value()).collect())
            }
            IndexKey::MaxKey => serde_json::Value::Null, // MaxKey is internal, return Null
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
    /// Whether this is a sparse index (only indexes documents where the field exists)
    pub sparse: bool,
    /// Estimated distinct key count for selectivity (0 = unknown)
    pub distinct_count: u64,
    /// Whether this index is currently being built (should be ignored by query planner)
    pub building: bool,
    /// Total number of keys in the index (for cost calculation)
    pub num_keys: u64,
    /// Whether this is a case-insensitive index
    pub case_insensitive: bool,
    /// Number of null key values in the index (for sparse index cost)
    pub null_count: u64,
    /// Ratio of documents with multikey arrays (0.0-1.0)
    /// Higher values indicate more index entries per document
    pub multikey_ratio: f32,
    /// Histogram for range query selectivity estimation
    /// Only populated for large indexes (100k+ entries) with single field
    pub histogram: Option<Histogram>,
    /// Most Common Values for equality selectivity estimation
    /// Only populated for indexes with 1000+ entries
    pub mcv: Option<MostCommonValues>,
}
