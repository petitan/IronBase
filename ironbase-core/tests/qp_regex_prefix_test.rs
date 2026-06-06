//! Query-planner correctness regression tests — PR4 (audit 2026-06-06, H+F+I).
//!
//! - H: prefix range upper bound was `prefix + U+10FFFF` *inclusive*, dropping a
//!   stored value `prefix + U+10FFFF + …`. Now a true half-open [prefix, succ).
//! - I: non-exact prefix regex (`^Hello.*`) count fell through to a full scan;
//!   now it uses the index range + post-filter (and explain is accurate).
//! - F: a `(?i)` regex over a case-insensitive index counted the raw key range
//!   without re-applying the pattern; Unicode case-folding (U+0130 etc.) differs
//!   from to_lowercase, so it over-counted. Now routed through the post-filter.

mod common;
use common::{assert_ci_index_matches_scan, assert_index_matches_scan};
use serde_json::json;

// ----- exact case-sensitive prefix: fast path must stay correct -----

#[test]
fn exact_cs_prefix_correct() {
    let docs = vec![
        json!({"name": "Hello"}),
        json!({"name": "Hello World"}),
        json!({"name": "Help"}),
        json!({"name": "hello"}),
        json!({"name": "Bye"}),
    ];
    // ^Hel → Hello, Hello World, Help → 3 (case-sensitive).
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^Hel"}}));
}

// ----- I: non-exact prefix regex -----

#[test]
fn non_exact_prefix_regex_correct() {
    let docs = vec![
        json!({"name": "Hello"}),
        json!({"name": "Hello World"}),
        json!({"name": "Help"}),
        json!({"name": "hello"}),
        json!({"name": "Bye"}),
    ];
    // ^Hello.* → Hello, Hello World → 2. count must equal find (not full-scan path).
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^Hello.*"}}));
}

#[test]
fn non_exact_prefix_with_charclass() {
    let docs = vec![
        json!({"name": "item1"}),
        json!({"name": "item2"}),
        json!({"name": "itemX"}),
        json!({"name": "other"}),
    ];
    // ^item[0-9] → item1, item2 → 2.
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^item[0-9]"}}));
}

// ----- H: value containing the maximum Unicode scalar after the prefix -----

#[test]
fn prefix_upper_bound_includes_max_scalar_suffix() {
    let docs = vec![
        json!({"name": "abc\u{10ffff}X"}),
        json!({"name": "abc\u{10ffff}"}),
        json!({"name": "abcdef"}),
        json!({"name": "abc"}),
        json!({"name": "abz"}),
        json!({"name": "abd"}),
    ];
    // ^abc → abc, abcdef, abc+U+10FFFF, abc+U+10FFFF+X → 4 (abd/abz excluded).
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^abc"}}));
}

#[test]
fn prefix_ending_in_max_scalar() {
    let docs = vec![
        json!({"name": "z\u{10ffff}"}),
        json!({"name": "z\u{10ffff}tail"}),
        json!({"name": "z"}),
    ];
    // Prefix itself ends in U+10FFFF (carry path in prefix_scan_upper_bound).
    assert_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "^z\u{10ffff}"}}));
}

// ----- F: (?i) regex over a case-insensitive index, Unicode case folding -----

#[test]
fn ci_regex_unicode_case_folding_correct() {
    let docs = vec![
        json!({"name": "\u{0130}stanbul"}), // İ (U+0130) — folds differently than to_lowercase
        json!({"name": "ice"}),
        json!({"name": "Item"}),
        json!({"name": "Apple"}),
    ];
    // (?i)^i — regex Unicode folding does NOT match U+0130 → ice, Item → 2.
    assert_ci_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "(?i)^i"}}));
}

#[test]
fn ci_regex_ascii_correct() {
    let docs = vec![
        json!({"name": "Apple"}),
        json!({"name": "apricot"}),
        json!({"name": "Banana"}),
    ];
    // (?i)^ap → Apple, apricot → 2.
    assert_ci_index_matches_scan(&docs, "name", &json!({"name": {"$regex": "(?i)^ap"}}));
}
