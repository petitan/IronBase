use super::rerank::strip_punctuation;
use super::*;
use serde_json::json;

#[test]
fn test_strip_punctuation() {
    assert_eq!(strip_punctuation("hello, world!"), "hello world");
    assert_eq!(strip_punctuation("a. b. c."), "a b c");
    assert_eq!(strip_punctuation("király   mesék"), "király mesék");
}

#[test]
fn test_strip_punctuation_leading_trailing() {
    assert_eq!(strip_punctuation("  hello  "), "hello");
    assert_eq!(strip_punctuation("...test..."), "test");
}

#[test]
fn test_id_to_string_string() {
    assert_eq!(id_to_string(&json!("abc")), Some("abc".to_string()));
}

#[test]
fn test_id_to_string_number() {
    assert_eq!(id_to_string(&json!(123)), Some("123".to_string()));
}

#[test]
fn test_id_to_string_uuid() {
    let id = json!("a1aaed76-f22c-472f-82cb-a9ed005cb624");
    assert_eq!(
        id_to_string(&id),
        Some("a1aaed76-f22c-472f-82cb-a9ed005cb624".to_string())
    );
}

#[test]
fn test_id_to_string_object() {
    let id = json!({"$oid": "507f1f77bcf86cd799439011"});
    assert!(id_to_string(&id).is_some());
}

// -------------------------------------------------------------------------
// Reranking Tests
// -------------------------------------------------------------------------

fn make_fused_result(id: &str, content: &str, heading: &str, rrf_score: f64) -> FusedResult {
    FusedResult {
        id: id.to_string(),
        doc: json!({
            "_id": id,
            "content": content,
            "heading": heading
        }),
        rrf_score,
        final_score: rrf_score,
        rerank_boost: 1.0,
        v_rank: 1,
        t_rank: 1,
        v_score: Some(0.9),
        t_score: Some(10.0),
    }
}

#[test]
fn test_rerank_exact_phrase_boost() {
    let mut results = vec![
        make_fused_result("doc1", "Some words about stuff", "Title", 0.01),
        make_fused_result(
            "doc2",
            "This document contains the exact phrase test query here",
            "Title",
            0.01,
        ),
    ];

    rerank_results(&mut results, "the exact phrase test query", "content", None);

    // doc2 should be boosted (contains exact phrase)
    assert!(results[0].id == "doc2");
    assert!(results[0].rerank_boost > 1.0);
}

#[test]
fn test_rerank_exact_phrase_ignores_punctuation() {
    let mut results = vec![
        make_fused_result(
            "doc1",
            "Some other content here that is long enough to avoid the short content penalty threshold of one hundred characters",
            "Title",
            0.01,
        ),
        make_fused_result(
            "doc2",
            "Ez a dokumentum tartalmazza: milyen lépései vannak a kalibrálásnak - itt van a kalibrálás leírása részletesen kifejtve",
            "Title",
            0.01,
        ),
    ];

    rerank_results(
        &mut results,
        "milyen lépései vannak a kalibrálásnak?",
        "content",
        None,
    );

    // doc2 should be boosted (phrase matches ignoring punctuation)
    assert!(results[0].id == "doc2");
    assert!(results[0].rerank_boost >= 1.5);
}

#[test]
fn test_rerank_short_content_penalty() {
    let mut results = vec![
        make_fused_result("doc1", "Short", "Title", 0.01),
        make_fused_result(
            "doc2",
            "This is a much longer content that exceeds the 100 character threshold and should not receive a penalty for being too short",
            "Title",
            0.01,
        ),
    ];

    rerank_results(&mut results, "test", "content", None);

    // doc1 should be penalized (content < 50 chars)
    let doc1 = results.iter().find(|r| r.id == "doc1").unwrap();
    let doc2 = results.iter().find(|r| r.id == "doc2").unwrap();
    assert!(doc1.rerank_boost < doc2.rerank_boost);
}

#[test]
fn test_rerank_title_match_boost() {
    let mut results = vec![
        FusedResult {
            id: "doc1".to_string(),
            doc: json!({
                "_id": "doc1",
                "content": "Ez egy hosszabb tartalom ami nem tartalmaz releváns szavakat de elég hosszú az ötven karakteres küszöbhöz",
                "title": "Nem kapcsolódó cím"
            }),
            rrf_score: 0.02,
            final_score: 0.02,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: Some(0.9),
            t_score: Some(10.0),
        },
        FusedResult {
            id: "doc2".to_string(),
            doc: json!({
                "_id": "doc2",
                "content": "Ez egy hosszabb tartalom ami nem tartalmaz releváns szavakat de elég hosszú az ötven karakteres küszöbhöz",
                "title": "Fékerőmérő kalibrálás és beállítás"
            }),
            rrf_score: 0.02,
            final_score: 0.02,
            rerank_boost: 1.0,
            v_rank: 2,
            t_rank: 2,
            v_score: Some(0.8),
            t_score: Some(8.0),
        },
    ];

    rerank_results(
        &mut results,
        "fékerőmérő kalibrálás",
        "content",
        Some("title"),
    );

    // doc2 should be boosted because title contains both query words
    assert_eq!(results[0].id, "doc2");
    assert!(results[0].rerank_boost > 1.0);
    let doc1 = results.iter().find(|r| r.id == "doc1").unwrap();
    assert!(doc1.rerank_boost <= 1.0 || doc1.rerank_boost < results[0].rerank_boost);
}

#[test]
fn test_rerank_title_match_partial() {
    let mut results = vec![FusedResult {
        id: "doc1".to_string(),
        doc: json!({
            "_id": "doc1",
            "content": "Tartalom ami elég hosszú ahhoz hogy ne kapjon rövid tartalom büntetést a rerankertől",
            "title": "Fékerőmérő javítás"  // 1 of 2 query words
        }),
        rrf_score: 0.02,
        final_score: 0.02,
        rerank_boost: 1.0,
        v_rank: 1,
        t_rank: 1,
        v_score: Some(0.9),
        t_score: Some(10.0),
    }];

    rerank_results(
        &mut results,
        "fékerőmérő kalibrálás",
        "content",
        Some("title"),
    );

    // Partial match: 1/2 words → boost = 1.0 + 0.5 * 0.5 = 1.25x
    assert!(results[0].rerank_boost >= 1.2);
    assert!(results[0].rerank_boost < 1.5);
}

#[test]
fn test_rerank_no_title_field() {
    let mut results = vec![FusedResult {
        id: "doc1".to_string(),
        doc: json!({
            "_id": "doc1",
            "content": "Tartalom ami elég hosszú ahhoz hogy ne kapjon rövid tartalom büntetést a rerankertől",
            "title": "Fékerőmérő kalibrálás"
        }),
        rrf_score: 0.02,
        final_score: 0.02,
        rerank_boost: 1.0,
        v_rank: 1,
        t_rank: 1,
        v_score: Some(0.9),
        t_score: Some(10.0),
    }];

    rerank_results(
        &mut results,
        "fékerőmérő kalibrálás",
        "content",
        None, // no title_field
    );

    // Without title_field, boost should be close to 1.0 (only density boost applies)
    assert!(results[0].rerank_boost < 1.4);
}

// -------------------------------------------------------------------------
// MMR Reordering Tests
// -------------------------------------------------------------------------

fn make_fused_with_embedding(
    id: &str,
    content: &str,
    embedding: Vec<f32>,
    score: f64,
) -> FusedResult {
    FusedResult {
        id: id.to_string(),
        doc: json!({
            "_id": id,
            "content": content,
            "embedding": embedding
        }),
        rrf_score: score,
        final_score: score,
        rerank_boost: 1.0,
        v_rank: 1,
        t_rank: 1,
        v_score: Some(0.9),
        t_score: Some(10.0),
    }
}

#[test]
fn test_mmr_selects_diverse_results() {
    let mut results = vec![
        make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
        make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
        make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
    ];

    let removed = mmr_reorder(&mut results, "embedding", 0.5, 2);

    assert_eq!(removed, 1);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id, "doc1");
    assert_eq!(results[1].id, "doc3");
}

#[test]
fn test_mmr_pure_relevance() {
    let mut results = vec![
        make_fused_with_embedding("doc1", "Content A", vec![1.0, 0.0, 0.0], 0.03),
        make_fused_with_embedding("doc2", "Content A copy", vec![1.0, 0.0, 0.0], 0.02),
        make_fused_with_embedding("doc3", "Content B", vec![0.0, 1.0, 0.0], 0.01),
    ];

    mmr_reorder(&mut results, "embedding", 1.0, 3);

    assert_eq!(results[0].id, "doc1");
    assert_eq!(results[1].id, "doc2");
    assert_eq!(results[2].id, "doc3");
}

#[test]
fn test_mmr_without_embeddings() {
    let mut results = vec![
        make_fused_result("doc1", "Content A", "Title", 0.03),
        make_fused_result("doc2", "Content B", "Title", 0.02),
        make_fused_result("doc3", "Content C", "Title", 0.01),
    ];

    let removed = mmr_reorder(&mut results, "embedding", 0.5, 2);

    assert_eq!(removed, 1);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].id, "doc1");
    assert_eq!(results[1].id, "doc2");
}

#[test]
fn test_mmr_limit_larger_than_results() {
    let mut results = vec![
        make_fused_with_embedding("doc1", "A", vec![1.0, 0.0], 0.02),
        make_fused_with_embedding("doc2", "B", vec![0.0, 1.0], 0.01),
    ];

    let removed = mmr_reorder(&mut results, "embedding", 0.5, 10);

    assert_eq!(removed, 0);
    assert_eq!(results.len(), 2);
}

// -------------------------------------------------------------------------
// Adjacent Chunk Merge Tests
// -------------------------------------------------------------------------

fn make_chunk(
    id: &str,
    doc_id: &str,
    chunk_index: u64,
    text: &str,
    start_char: u64,
    end_char: u64,
    score: f64,
) -> FusedResult {
    FusedResult {
        id: id.to_string(),
        doc: json!({
            "_id": id,
            "doc_id": doc_id,
            "chunk_index": chunk_index,
            "content": text,
            "start_char": start_char,
            "end_char": end_char
        }),
        rrf_score: score,
        final_score: score,
        rerank_boost: 1.0,
        v_rank: 1,
        t_rank: 1,
        v_score: Some(0.9),
        t_score: Some(10.0),
    }
}

#[test]
fn test_merge_adjacent_basic() {
    // Two adjacent chunks from same doc → merge into 1
    let mut results = vec![
        make_chunk("c6", "docA", 6, "Hello world overlap_", 600, 700, 0.03),
        make_chunk("c7", "docA", 7, "overlap_region end text", 680, 800, 0.02),
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");

    assert_eq!(removed, 1);
    assert_eq!(results.len(), 1);
    // Best score kept
    assert!((results[0].final_score - 0.03).abs() < 1e-10);
    // Metadata
    assert_eq!(results[0].doc["chunk_merged"], true);
    assert_eq!(results[0].doc["chunks_in_merge"], 2);
    assert_eq!(results[0].doc["chunk_index"], 6);
    assert_eq!(results[0].doc["start_char"], 600);
    assert_eq!(results[0].doc["end_char"], 800);
}

#[test]
fn test_merge_distant_chunks_preserved() {
    // Same doc_id but non-adjacent chunk_index → NOT merged
    let mut results = vec![
        make_chunk("c10", "docA", 10, "Chunk 10 text", 1000, 1100, 0.03),
        make_chunk("c15", "docA", 15, "Chunk 15 text", 1500, 1600, 0.02),
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");

    assert_eq!(removed, 0);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_merge_three_consecutive() {
    // Three consecutive chunks → merge into 1
    // Realistic overlap: 4 chars overlap out of ~14 char chunks
    let mut results = vec![
        make_chunk("c5", "docA", 5, "AAAA BBBB CCCC", 0, 14, 0.01),
        make_chunk("c6", "docA", 6, "CCCC DDDD EEEE", 10, 24, 0.03),
        make_chunk("c7", "docA", 7, "EEEE FFFF GGGG", 20, 34, 0.02),
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");

    assert_eq!(removed, 2);
    assert_eq!(results.len(), 1);
    // Best score from run (c6 = 0.03)
    assert!((results[0].final_score - 0.03).abs() < 1e-10);
    assert_eq!(results[0].doc["chunks_in_merge"], 3);
    assert_eq!(results[0].doc["start_char"], 0);
    assert_eq!(results[0].doc["end_char"], 34);
    // Text should be merged with overlap removed
    let merged_text = results[0].doc["content"].as_str().unwrap();
    assert_eq!(merged_text, "AAAA BBBB CCCC DDDD EEEE FFFF GGGG");
}

/// Regression for #64: merge must only touch the text field it is given;
/// other fields (e.g. `title`, identical across chunks) must stay intact.
/// The original bug merged a mis-resolved field (`title`), concatenating it
/// N times while leaving content unmerged.
#[test]
fn test_merge_leaves_other_fields_intact() {
    let mut results = vec![
        FusedResult {
            id: "c0".into(),
            doc: json!({
                "_id": "c0", "doc_id": "docA", "chunk_index": 0,
                "content": "AAAA BBBB", "title": "BKV Zrt árajánlat",
                "start_char": 0, "end_char": 9
            }),
            rrf_score: 0.02,
            final_score: 0.02,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: None,
            t_score: Some(1.0),
        },
        FusedResult {
            id: "c1".into(),
            doc: json!({
                "_id": "c1", "doc_id": "docA", "chunk_index": 1,
                "content": "BBBB CCCC", "title": "BKV Zrt árajánlat",
                "start_char": 5, "end_char": 14
            }),
            rrf_score: 0.03,
            final_score: 0.03,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: None,
            t_score: Some(1.0),
        },
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");
    assert_eq!(removed, 1);
    assert_eq!(results.len(), 1);
    // Content is merged across both chunks (overlap removed)...
    assert_eq!(
        results[0].doc["content"].as_str().unwrap(),
        "AAAA BBBB CCCC"
    );
    // ...but the title is the single stored value, NOT concatenated (#64).
    assert_eq!(
        results[0].doc["title"].as_str().unwrap(),
        "BKV Zrt árajánlat"
    );
}

/// #63: when adjacent table chunks (each carrying the propagated header) are
/// merged, the header must appear exactly once in the merged text.
#[test]
fn test_merge_dedups_propagated_table_header() {
    let header = "| Megnevezés | Ár |\n|---|---|";
    let mut results = vec![
        FusedResult {
            id: "c0".into(),
            doc: json!({
                "_id": "c0", "doc_id": "docA", "chunk_index": 0,
                "content": format!("{}\n| PEF-35 | 1750000 |", header),
                "start_char": 0, "end_char": 10
            }),
            rrf_score: 0.02,
            final_score: 0.02,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: None,
            t_score: Some(1.0),
        },
        FusedResult {
            id: "c1".into(),
            doc: json!({
                // Continuation chunk: header prepended by propagation, recorded
                // in the table_header field so the merge strips it exactly.
                "_id": "c1", "doc_id": "docA", "chunk_index": 1,
                "content": format!("{}\n| PEF-50 | 2100000 |", header),
                "table_header": header,
                "start_char": 10, "end_char": 20
            }),
            rrf_score: 0.03,
            final_score: 0.03,
            rerank_boost: 1.0,
            v_rank: 1,
            t_rank: 1,
            v_score: None,
            t_score: Some(1.0),
        },
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");
    assert_eq!(removed, 1);
    assert_eq!(results.len(), 1);
    let merged = results[0].doc["content"].as_str().unwrap();
    // Header (separator) appears exactly once; both data rows survive.
    assert_eq!(merged.matches("|---|---|").count(), 1, "merged: {merged}");
    assert!(
        merged.contains("PEF-35") && merged.contains("PEF-50"),
        "merged: {merged}"
    );
}

#[test]
fn test_merge_no_doc_id() {
    // Results without doc_id → untouched
    let mut results = vec![
        make_fused_result("doc1", "Content A", "Title", 0.03),
        make_fused_result("doc2", "Content B", "Title", 0.02),
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");

    assert_eq!(removed, 0);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_merge_mixed_docs() {
    // doc_A chunk 6+7 (adjacent) + doc_B chunk 3 (alone) → doc_A merges, doc_B stays
    let mut results = vec![
        make_chunk("a6", "docA", 6, "A chunk six_", 600, 700, 0.03),
        make_chunk("a7", "docA", 7, "six_A chunk seven", 680, 800, 0.02),
        make_chunk("b3", "docB", 3, "B chunk three", 300, 400, 0.025),
    ];

    let removed = merge_adjacent_chunks(&mut results, "content");

    assert_eq!(removed, 1);
    assert_eq!(results.len(), 2);
    // docA merged, docB untouched
    let has_doc_a = results
        .iter()
        .any(|r| r.doc.get("doc_id").and_then(|v| v.as_str()) == Some("docA"));
    let has_doc_b = results
        .iter()
        .any(|r| r.doc.get("doc_id").and_then(|v| v.as_str()) == Some("docB"));
    assert!(has_doc_a);
    assert!(has_doc_b);
}

#[test]
fn test_merge_empty() {
    let mut results: Vec<FusedResult> = vec![];
    let removed = merge_adjacent_chunks(&mut results, "content");
    assert_eq!(removed, 0);
    assert_eq!(results.len(), 0);
}

/// docs/SEARCH_CONTEXT_BUDGET_BUG.md: a table's consecutive matching
/// chunks merged into one passage of 57 267 chars. A grouped merge now
/// stays within `MAX_MERGED_PASSAGE_CHARS` around its best chunk, the rest
/// of the run forms further passages, and the best chunk's offset is kept.
#[test]
fn grouped_merge_is_bounded_around_the_best_chunk() {
    // 12 consecutive chunks of ~1000 bytes, best-first order: chunk 6 first
    let order = [6u64, 2, 9, 0, 11, 5, 7, 1, 3, 4, 8, 10];
    let mut chunks: Vec<Value> = order
        .iter()
        .map(|&ci| {
            json!({"doc_id": "t", "chunk_index": ci,
                   "content": format!("<{ci:02}>{}", "x".repeat(996))})
        })
        .collect();
    merge_adjacent_values(&mut chunks, "content");

    let texts: Vec<&str> = chunks
        .iter()
        .map(|c| c["content"].as_str().unwrap())
        .collect();
    for t in &texts {
        assert!(
            t.len() <= MAX_MERGED_PASSAGE_CHARS,
            "passage of {} bytes",
            t.len()
        );
    }
    // Every chunk's text is kept exactly once
    let all: String = texts.concat();
    for ci in 0..12 {
        assert_eq!(all.matches(&format!("<{ci:02}>")).count(), 1, "chunk {ci}");
    }
    // The best chunk leads the first passage's window and its offset is kept
    let first = &chunks[0];
    let offset = first[MERGE_BEST_OFFSET_KEY].as_u64().unwrap() as usize;
    assert!(first["content"].as_str().unwrap()[offset..].starts_with("<06>"));
    assert_eq!(
        first["chunks_in_merge"],
        json!(3),
        "6 and its neighbours 5, 7"
    );
}

/// The flat merge (Rhai / `hybrid_search` results) keeps whole runs and
/// adds no internal key
#[test]
fn flat_merge_is_unbounded() {
    let text = "y".repeat(1000);
    let mut results: Vec<FusedResult> = (0..6u64)
        .map(|ci| {
            let start = ci * 1000;
            make_chunk(
                &format!("c{ci}"),
                "t",
                ci,
                &text,
                start,
                start + 1000,
                1.0 - ci as f64 * 0.1,
            )
        })
        .collect();
    assert_eq!(merge_adjacent_chunks(&mut results, "content"), 5);
    assert_eq!(results[0].doc["content"].as_str().unwrap().len(), 6000);
    assert!(results[0].doc.get(MERGE_BEST_OFFSET_KEY).is_none());
}

// lift_common_fields (#69 — group_by_document doc-level field promotion)

#[test]
fn test_lift_common_fields_promotes_identical_and_drops_from_chunks() {
    let mut chunks = vec![
        json!({"doc_id":"a","title":"T","year":2026,"content":"c1","chunk_index":0,"_text_score":6.2}),
        json!({"doc_id":"a","title":"T","year":2026,"content":"c2","chunk_index":1,"_text_score":5.1}),
        json!({"doc_id":"a","title":"T","year":2026,"content":"c3","chunk_index":2,"_text_score":4.0}),
    ];
    let lifted = lift_common_fields(&mut chunks);

    // doc_id is explicitly skipped (group root carries it).
    assert!(!lifted.contains_key("doc_id"));
    // Identical-everywhere keys lifted.
    assert_eq!(lifted["title"], json!("T"));
    assert_eq!(lifted["year"], json!(2026));
    // Varying keys stay in each chunk.
    assert!(!lifted.contains_key("content"));
    assert!(!lifted.contains_key("chunk_index"));
    assert!(!lifted.contains_key("_text_score"));
    // Lifted keys removed from each chunk → no duplication.
    for c in &chunks {
        assert!(c.get("title").is_none(), "title still in chunk: {c}");
        assert!(c.get("year").is_none(), "year still in chunk: {c}");
        assert!(
            c.get("content").is_some(),
            "content missing from chunk: {c}"
        );
    }
}

#[test]
fn test_lift_common_fields_skips_single_chunk_group() {
    // With one chunk, "all match" is vacuously true on every key — lifting
    // would strand chunks[0] empty. Gated: no lift, chunk stays self-contained.
    let mut chunks = vec![json!({"doc_id":"x","title":"T","content":"only"})];
    let lifted = lift_common_fields(&mut chunks);
    assert!(lifted.is_empty(), "single-chunk groups must not lift");
    // chunk[0] retains all its fields (title, content) — clients iterating
    // group.chunks for snippet text get something to render.
    assert_eq!(chunks[0]["title"], json!("T"));
    assert_eq!(chunks[0]["content"], json!("only"));
}

#[test]
fn test_lift_common_fields_skips_chunk_level_engine_metadata() {
    // Chunk-level engine metadata (`_text_score`, `chunk_merged`, etc.) MUST NOT
    // lift even if it happens to be identical across all chunks (BM25 tie, all
    // merged, etc.). Lifting would falsely promote chunk semantics to the doc.
    let mut chunks = vec![
        json!({
            "doc_id":"a", "title":"T",
            "chunk_index":3, "chunk_merged":true, "chunks_in_merge":2,
            "_text_score":5.0, "_final_score":0.03, "_text_rank":1,
            "table_header":"| H |\n|---|",
            "start_char":0, "end_char":100,
            "content":"first"
        }),
        json!({
            "doc_id":"a", "title":"T",
            "chunk_index":3, "chunk_merged":true, "chunks_in_merge":2,
            "_text_score":5.0, "_final_score":0.03, "_text_rank":1,
            "table_header":"| H |\n|---|",
            "start_char":0, "end_char":100,
            "content":"second"
        }),
    ];
    let lifted = lift_common_fields(&mut chunks);
    // Only the genuinely doc-level field lifts.
    assert_eq!(lifted["title"], json!("T"));
    // Chunk-level engine/structural keys stay in chunks, no matter how identical.
    for k in [
        "chunk_index",
        "chunk_merged",
        "chunks_in_merge",
        "_text_score",
        "_final_score",
        "_text_rank",
        "table_header",
        "start_char",
        "end_char",
    ] {
        assert!(
            !lifted.contains_key(k),
            "chunk-level key '{k}' must not lift"
        );
        for c in &chunks {
            assert!(
                c.get(k).is_some(),
                "chunk-level '{k}' missing from chunk: {c}"
            );
        }
    }
}

#[test]
fn test_lift_common_fields_skips_chunk_payload_defaults() {
    // Defensive: duplicate ingest can leave every chunk with identical
    // `embedding` (vector) and `content` (text) — these must NOT lift, even
    // when value-equal across the whole group. Lifting `embedding` would
    // bloat the response; lifting `content` is semantically wrong.
    let mut chunks = vec![
        json!({
            "doc_id":"a", "title":"T",
            "content":"identical text",
            "embedding":[0.1,0.2,0.3]
        }),
        json!({
            "doc_id":"a", "title":"T",
            "content":"identical text",
            "embedding":[0.1,0.2,0.3]
        }),
    ];
    let lifted = lift_common_fields(&mut chunks);
    assert_eq!(lifted["title"], json!("T"));
    for k in ["content", "embedding"] {
        assert!(
            !lifted.contains_key(k),
            "chunk-payload key '{k}' must not lift"
        );
        for c in &chunks {
            assert!(c.get(k).is_some(), "'{k}' should remain in chunk: {c}");
        }
    }
}

#[test]
fn test_lift_common_fields_skips_engine_reserved_keys() {
    // A user field happening to be named `best_score`/`chunk_count`/`chunks`
    // with identical values across chunks must NOT be lifted — those names
    // are reserved for the engine at the group root and would overwrite it.
    let mut chunks = vec![
        json!({"doc_id":"a","title":"T","best_score":0.99,"chunk_count":7,"chunks":["nested"],"content":"c1"}),
        json!({"doc_id":"a","title":"T","best_score":0.99,"chunk_count":7,"chunks":["nested"],"content":"c2"}),
    ];
    let lifted = lift_common_fields(&mut chunks);
    assert_eq!(lifted["title"], json!("T"));
    for k in ["doc_id", "best_score", "chunk_count", "chunks"] {
        assert!(
            !lifted.contains_key(k),
            "engine-reserved key '{k}' must not lift"
        );
        // ...and stays untouched in each chunk.
        for c in &chunks {
            assert!(
                c.get(k).is_some(),
                "engine-reserved '{k}' missing from chunk: {c}"
            );
        }
    }
}

#[test]
fn test_lift_common_fields_empty_input() {
    let mut chunks: Vec<Value> = Vec::new();
    let lifted = lift_common_fields(&mut chunks);
    assert!(lifted.is_empty());
}
