//! P9 (property-based testing plan): fulltext and vector search.
//!
//! Fulltext (BM25, k1 = 1.2, b = 0.75, IDF = ln(1 + N/df), as documented):
//! - the top-k equals a raw BM25 oracle over the same tokens;
//! - add/remove is symmetric: indexing A and B, then removing B, answers as
//!   indexing A alone (an update as remove + insert);
//! - a document is found by any word it keeps after tokenization, whatever
//!   case or Unicode form (composed/decomposed accents) the query uses.
//!
//! HNSW (statistical bound, small graphs):
//! - recall@k against brute-force cosine stays above a bound;
//! - a removed vector is never returned, also before an orphan rebuild;
//! - results are distinct, at most k, ordered by score, and `len` counts the
//!   live vectors.

use ironbase_core::document::DocumentId;
use ironbase_core::fulltext::{tokenize, tokenize_unique};
use ironbase_core::vector::{HnswIndex, VectorIndexConfig};
use ironbase_core::{FtsLanguage, FtsOptions, FulltextIndex};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use unicode_normalization::UnicodeNormalization;

fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

// ============================================================================
// Fulltext
// ============================================================================

/// Words: Hungarian and English inflections (so stemming merges some),
/// accented letters, stop words, short words, punctuation
fn arb_word() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(vec![
            "király", "királyok", "királynak", "ház", "házból", "házak", "kert", "kertben",
            "őrség", "őrök", "tűz", "tüzek", "run", "running", "runs", "house", "houses",
            "a", "az", "és", "the", "of", "x", "ab", "Űrhajó", "ŐRSÉG", "Árvíztűrő",
        ])
        .prop_map(String::from),
        1 => "[a-zéőű]{2,7}",
        1 => prop::sample::select(vec!["király,", "(ház)", "kert.", "--", "tűz!"]).prop_map(String::from),
    ]
}

fn arb_text() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_word(), 0..12).prop_map(|w| w.join(" "))
}

fn arb_language() -> impl Strategy<Value = FtsLanguage> {
    prop::sample::select(vec![
        FtsLanguage::Hungarian,
        FtsLanguage::English,
        FtsLanguage::None,
    ])
}

/// The raw BM25 oracle: scores of every document matching a query token
fn bm25_oracle(docs: &BTreeMap<i64, Vec<String>>, query: &[String]) -> Vec<(i64, f64)> {
    let n = docs.len() as f64;
    if n == 0.0 {
        return Vec::new();
    }
    let total_len: usize = docs.values().map(|t| t.len()).sum();
    let avgdl = total_len as f64 / n;
    let mut scores: BTreeMap<i64, f64> = BTreeMap::new();
    for token in query {
        let df = docs.values().filter(|t| t.contains(token)).count();
        if df == 0 {
            continue;
        }
        let idf = (1.0 + n / df as f64).ln();
        for (id, tokens) in docs {
            let tf = tokens.iter().filter(|t| *t == token).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let dl = tokens.len() as f64;
            let sat = if avgdl > 0.0 {
                let norm = 1.0 - 0.75 + 0.75 * (dl / avgdl);
                (2.2 * tf) / (1.2 * norm + tf)
            } else {
                (2.2 * tf) / (1.2 + tf)
            };
            *scores.entry(*id).or_default() += sat * idf;
        }
    }
    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
    ranked
}

/// The model of the indexed documents: a text without tokens (empty, or
/// only stop words and short words) is not indexed, and so counts neither
/// in N nor in the average length (as the engine)
fn index_model(docs: &mut BTreeMap<i64, Vec<String>>, id: i64, tokens: Vec<String>) {
    if tokens.is_empty() {
        docs.remove(&id);
    } else {
        docs.insert(id, tokens);
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

fn id_of(id: &DocumentId) -> i64 {
    match id {
        DocumentId::Int(n) => *n,
        other => panic!("unexpected id {other:?}"),
    }
}

/// Engine results against the oracle: same scores in the same order; ties
/// may cut the top-k at a different document, so ids are compared by score
fn check_search(
    index: &FulltextIndex,
    docs: &BTreeMap<i64, Vec<String>>,
    query: &str,
    options: &FtsOptions,
    limit: usize,
    why: &str,
) -> Result<(), TestCaseError> {
    let query_tokens = tokenize_unique(query, options);
    let expected = bm25_oracle(docs, &query_tokens);
    let got = index.search(query, limit, 0, None);
    prop_assert_eq!(
        got.len(),
        expected.len().min(limit),
        "{}: result count, query {:?}",
        why,
        query
    );
    let score_of: HashMap<i64, f64> = expected.iter().cloned().collect();
    for (rank, (hit, (_, want))) in got.iter().zip(&expected).enumerate() {
        let id = id_of(&hit.doc_id);
        prop_assert!(
            close(hit.score, *want),
            "{}: rank {} score {} vs {}, query {:?}",
            why,
            rank,
            hit.score,
            want,
            query
        );
        prop_assert!(
            score_of.get(&id).is_some_and(|s| close(*s, hit.score)),
            "{}: doc {} scored {} (oracle {:?}), query {:?}",
            why,
            id,
            hit.score,
            score_of.get(&id),
            query
        );
    }
    let ids: BTreeSet<i64> = got.iter().map(|h| id_of(&h.doc_id)).collect();
    prop_assert_eq!(ids.len(), got.len(), "{}: repeated document", why);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(128)))]

    /// The engine's top-k is the oracle's BM25 ranking
    #[test]
    fn fulltext_ranks_like_bm25(
        language in arb_language(),
        texts in prop::collection::vec(arb_text(), 0..25),
        queries in prop::collection::vec(arb_text(), 1..5),
        limit in 1usize..12,
    ) {
        let options = FtsOptions::new(language);
        let mut index = FulltextIndex::new("ft", "body", options.clone());
        let mut docs = BTreeMap::new();
        for (i, text) in texts.iter().enumerate() {
            index.insert(&DocumentId::Int(i as i64), text).unwrap();
            index_model(&mut docs, i as i64, tokenize(text, &options));
        }
        for query in &queries {
            check_search(&index, &docs, query, &options, limit, "fresh")?;
        }
    }

    /// Removing documents answers as never having indexed them; an update
    /// (or an `insert` on an indexed id) answers as the new text alone, also
    /// a text without tokens
    #[test]
    fn fulltext_add_remove_is_symmetric(
        language in arb_language(),
        kept in prop::collection::vec(arb_text(), 0..15),
        removed in prop::collection::vec(arb_text(), 1..10),
        updated in prop::collection::vec(
            (any::<prop::sample::Index>(), prop_oneof![3 => arb_text(), 1 => Just(String::from("a az the"))]),
            0..6,
        ),
        queries in prop::collection::vec(arb_text(), 1..5),
    ) {
        let options = FtsOptions::new(language);
        let mut index = FulltextIndex::new("ft", "body", options.clone());
        let mut docs = BTreeMap::new();
        for (i, text) in kept.iter().enumerate() {
            index.insert(&DocumentId::Int(i as i64), text).unwrap();
            index_model(&mut docs, i as i64, tokenize(text, &options));
        }
        let base = kept.len() as i64;
        for (i, text) in removed.iter().enumerate() {
            index.insert(&DocumentId::Int(base + i as i64), text).unwrap();
        }
        for i in 0..removed.len() {
            index.remove(&DocumentId::Int(base + i as i64)).unwrap();
        }
        if !kept.is_empty() {
            for (n, (pick, text)) in updated.iter().enumerate() {
                let id = pick.index(kept.len()) as i64;
                // `insert` on an indexed id replaces it too
                if n % 2 == 0 {
                    index.update(&DocumentId::Int(id), text).unwrap();
                } else {
                    index.insert(&DocumentId::Int(id), text).unwrap();
                }
                index_model(&mut docs, id, tokenize(text, &options));
            }
        }
        prop_assert_eq!(index.doc_count(), docs.len());
        for query in &queries {
            check_search(&index, &docs, query, &options, 50, "after remove/update")?;
        }
    }

    /// On disk: flushed, reloaded lazily, then changed again, the index
    /// still answers as the oracle (posting lists read from the file, through
    /// the posting cache, merged with changes made after the load)
    #[test]
    fn fulltext_on_disk_ranks_like_bm25(
        language in arb_language(),
        texts in prop::collection::vec(arb_text(), 0..20),
        later in prop::collection::vec(arb_text(), 0..6),
        removed in prop::collection::btree_set(0usize..20, 0..6),
        queries in prop::collection::vec(arb_text(), 1..5),
    ) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("i.ftidx");
        let options = FtsOptions::new(language);
        let mut index = FulltextIndex::new_with_storage("ft", "body", options.clone(), path.clone()).unwrap();
        let mut docs = BTreeMap::new();
        for (i, text) in texts.iter().enumerate() {
            index.insert(&DocumentId::Int(i as i64), text).unwrap();
            index_model(&mut docs, i as i64, tokenize(text, &options));
        }
        index.flush().unwrap();
        for query in &queries {
            check_search(&index, &docs, query, &options, 50, "flushed")?;
        }
        drop(index);

        let mut index = FulltextIndex::load_from_file(path).unwrap();
        for query in &queries {
            // Twice: the second answer comes from the posting cache
            check_search(&index, &docs, query, &options, 50, "reloaded")?;
            check_search(&index, &docs, query, &options, 50, "reloaded, cached")?;
        }
        for i in removed.iter().filter(|i| **i < texts.len()) {
            index.remove(&DocumentId::Int(*i as i64)).unwrap();
            docs.remove(&(*i as i64));
        }
        let base = texts.len() as i64;
        for (i, text) in later.iter().enumerate() {
            index.insert(&DocumentId::Int(base + i as i64), text).unwrap();
            index_model(&mut docs, base + i as i64, tokenize(text, &options));
        }
        prop_assert_eq!(index.doc_count(), docs.len());
        for query in &queries {
            check_search(&index, &docs, query, &options, 50, "reloaded, then changed")?;
        }
    }

    /// A document is found by each word it keeps after tokenization, asked
    /// in upper case or with decomposed accents (NFD)
    #[test]
    fn fulltext_finds_a_document_by_its_own_words(
        language in arb_language(),
        texts in prop::collection::vec(arb_text(), 1..10),
        upper in any::<bool>(),
        decompose in any::<bool>(),
    ) {
        let options = FtsOptions::new(language);
        let mut index = FulltextIndex::new("ft", "body", options.clone());
        for (i, text) in texts.iter().enumerate() {
            index.insert(&DocumentId::Int(i as i64), text).unwrap();
        }
        for (i, text) in texts.iter().enumerate() {
            for word in text.split_whitespace() {
                if tokenize(word, &options).is_empty() {
                    continue; // a stop word or too short: not indexed
                }
                let mut query = if upper { word.to_uppercase() } else { word.to_string() };
                if decompose {
                    query = query.nfd().collect();
                }
                let hits = index.search(&query, 1000, 0, None);
                prop_assert!(
                    hits.iter().any(|h| h.doc_id == DocumentId::Int(i as i64)),
                    "{:?}: doc {} ({:?}) not found by {:?}",
                    language,
                    i,
                    text,
                    query
                );
            }
        }
    }
}

// ============================================================================
// HNSW
// ============================================================================

const DIM: usize = 8;

fn arb_vectors() -> impl Strategy<Value = Vec<Vec<f32>>> {
    prop::collection::vec(prop::collection::vec(-1.0f32..1.0, DIM), 1..150).prop_map(|vs| {
        vs.into_iter()
            .filter(|v| v.iter().any(|x| x.abs() > 1e-3))
            .collect()
    })
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    dot / (na * nb)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases(64)))]

    /// Recall@k against brute force stays high; removed vectors never come
    /// back; results are distinct, ordered, at most k; `len` is the live count
    #[test]
    fn hnsw_recall_and_removal(
        vectors in arb_vectors(),
        queries in prop::collection::vec(prop::collection::vec(-1.0f32..1.0, DIM), 1..10),
        removed in prop::collection::btree_set(0usize..150, 0..40),
        k in 1usize..10,
    ) {
        let mut index = HnswIndex::new(VectorIndexConfig { dim: DIM, ..Default::default() });
        for (i, v) in vectors.iter().enumerate() {
            index.insert(&i.to_string(), v).unwrap();
        }
        let removed: BTreeSet<usize> = removed.into_iter().filter(|i| *i < vectors.len()).collect();
        for i in &removed {
            index.remove(&i.to_string());
        }
        let live: Vec<usize> = (0..vectors.len()).filter(|i| !removed.contains(i)).collect();
        prop_assert_eq!(index.len(), live.len(), "len counts live vectors");

        let mut found = 0usize;
        let mut wanted = 0usize;
        for rebuilt in [false, true] {
            if rebuilt {
                index.rebuild().unwrap();
                prop_assert_eq!(index.orphan_count(), 0);
                prop_assert_eq!(index.len(), live.len(), "len after rebuild");
            }
            for query in &queries {
                if query.iter().all(|x| x.abs() < 1e-3) {
                    continue;
                }
                let hits = index.search(query, k);
                prop_assert!(hits.len() <= k);
                prop_assert_eq!(hits.len(), k.min(live.len()), "fewer results than live vectors allow (rebuilt {})", rebuilt);
                let ids: Vec<usize> = hits.iter().map(|h| h.id.parse().unwrap()).collect();
                let distinct: BTreeSet<usize> = ids.iter().cloned().collect();
                prop_assert_eq!(distinct.len(), ids.len(), "repeated id");
                prop_assert!(ids.iter().all(|i| !removed.contains(i)), "removed vector returned: {:?}", ids);
                prop_assert!(hits.windows(2).all(|w| w[0].score >= w[1].score - 1e-6), "not ordered by score");

                let mut exact: Vec<(usize, f64)> = live.iter().map(|&i| (i, cosine(query, &vectors[i]))).collect();
                exact.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                // Ties at the cut: any id scoring at least the k-th score counts
                let cut = exact.get(k.min(exact.len()).saturating_sub(1)).map_or(f64::MIN, |e| e.1);
                found += ids.iter().filter(|i| cosine(query, &vectors[**i]) >= cut - 1e-6).count();
                wanted += k.min(live.len());
            }
        }
        if wanted > 0 {
            let recall = found as f64 / wanted as f64;
            prop_assert!(recall >= 0.9, "recall@{} {:.3} ({} vectors, {} removed)", k, recall, vectors.len(), removed.len());
        }
    }
}
