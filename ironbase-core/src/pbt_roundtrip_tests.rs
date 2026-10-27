// ironbase-core/src/pbt_roundtrip_tests.rs
// P5 (property-based testing plan): what is written reads back bit for bit,
// and damaged bytes are rejected or tolerated, never a panic.
//
// One property pair per on-disk format: stored documents (file storage,
// reopened), WAL entries, B+ tree index files (eager and lazy load), HNSW
// index bytes and V4 fulltext posting lists. The "mutation" half writes the
// format, damages it, and reads it back.

#[cfg(test)]
mod roundtrip_properties {
    use crate::database::DatabaseCore;
    use crate::document::DocumentId;
    use crate::fulltext::{decode_token_entries, encode_token_entries, FTIDX_VERSION_V4};
    use crate::index::{BPlusTree, IndexKey, OrderedFloat, RangeQueryMode, ScanOrder};
    use crate::storage::StorageEngine;
    use crate::vector::{HnswIndex, VectorIndexConfig};
    use crate::wal::{WALEntry, WALEntryType};
    use proptest::prelude::*;
    use serde_json::{json, Map, Value};
    use std::collections::HashMap;
    use std::io::{Read, Seek, SeekFrom, Write};

    fn cases(default: u32) -> u32 {
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    /// Floats that do not survive a naive print → parse, signed zero, a
    /// subnormal, and any finite float
    fn arb_float() -> impl Strategy<Value = f64> {
        prop_oneof![
            2 => prop::sample::select(vec![
                0.1, -0.0, 1e-310, 123456789.12345679, 9007199254740993.0, 1e300, 5e-324,
            ]),
            1 => any::<f64>().prop_filter("finite", |f| f.is_finite()),
        ]
    }

    /// Any JSON value: escapes, control characters and non-BMP text in
    /// strings and keys, integers at the i64/u64 edges, nested containers
    fn arb_json() -> impl Strategy<Value = Value> {
        let text = prop_oneof![
            3 => any::<String>(),
            1 => prop::sample::select(vec!["", "\"", "\\", "\u{0}", "\n\t", "ő", "😀", "\u{2028}"])
                .prop_map(String::from),
        ];
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            prop::sample::select(vec![u64::MAX, i64::MAX as u64 + 1]).prop_map(Value::from),
            arb_float().prop_map(|f| json!(f)),
            text.clone().prop_map(Value::from),
        ];
        leaf.prop_recursive(3, 24, 4, move |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                prop::collection::btree_map(text.clone(), inner, 0..4)
                    .prop_map(|m| Value::Object(m.into_iter().collect())),
            ]
        })
    }

    /// A document's fields: names that are not reserved (`_id`, `_tombstone`,
    /// `_collection`) and contain no `.`, `$` or NUL
    fn arb_fields() -> impl Strategy<Value = Map<String, Value>> {
        prop::collection::btree_map("[a-zA-Zőé😀 ]{1,6}", arb_json(), 0..5)
            .prop_map(|m| m.into_iter().collect())
    }

    /// Flip, overwrite or truncate one position of `bytes`
    fn damage(mut bytes: Vec<u8>, at: prop::sample::Index, how: u8) -> Vec<u8> {
        if bytes.is_empty() {
            return bytes;
        }
        let i = at.index(bytes.len());
        match how % 3 {
            0 => bytes[i] ^= 1 << (how % 8),
            1 => bytes[i] = how,
            _ => bytes.truncate(i),
        }
        bytes
    }

    // ------------------------------------------------------------------
    // Stored documents
    // ------------------------------------------------------------------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(cases(32)))]

        /// Documents read back exactly as inserted, before and after reopen
        #[test]
        fn documents_read_back_exactly(docs in prop::collection::vec(arb_fields(), 1..8)) {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("r.mlite");
            let mut expected = Vec::new();
            {
                let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
                for fields in &docs {
                    let map: HashMap<String, Value> = fields.clone().into_iter().collect();
                    let id = db.insert_one("r", map).unwrap();
                    let mut doc = fields.clone();
                    doc.insert("_id".into(), serde_json::to_value(&id).unwrap());
                    expected.push(Value::Object(doc));
                }
                let found = db.get_collection("r").unwrap().find(&json!({})).unwrap();
                prop_assert_eq!(by_id(found), by_id(expected.clone()));
                db.close().unwrap();
            }
            let db = DatabaseCore::<StorageEngine>::open(&path).unwrap();
            let found = db.get_collection("r").unwrap().find(&json!({})).unwrap();
            prop_assert_eq!(by_id(found), by_id(expected));
            db.close().unwrap();
        }
    }

    fn by_id(docs: Vec<Value>) -> std::collections::BTreeMap<String, String> {
        // Compared as stored text: `1` and `1.0`, `0.0` and `-0.0` differ
        docs.into_iter()
            .map(|d| (d["_id"].to_string(), canonical(&d)))
            .collect()
    }

    /// JSON text with object keys sorted (key order is not part of a value)
    fn canonical(value: &Value) -> String {
        match value {
            Value::Object(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                let inner: Vec<String> = entries
                    .into_iter()
                    .map(|(k, v)| format!("{}:{}", Value::from(k.as_str()), canonical(v)))
                    .collect();
                format!("{{{}}}", inner.join(","))
            }
            Value::Array(items) => {
                format!(
                    "[{}]",
                    items.iter().map(canonical).collect::<Vec<_>>().join(",")
                )
            }
            other => other.to_string(),
        }
    }

    // ------------------------------------------------------------------
    // WAL entries
    // ------------------------------------------------------------------

    fn arb_wal_entry() -> impl Strategy<Value = WALEntry> {
        let entry_type = prop::sample::select(vec![
            WALEntryType::Begin,
            WALEntryType::Operation,
            WALEntryType::Commit,
            WALEntryType::Abort,
            WALEntryType::IndexChange,
            WALEntryType::MetadataSnapshot,
        ]);
        (
            any::<u64>(),
            entry_type,
            prop::collection::vec(any::<u8>(), 0..64),
        )
            .prop_map(|(tx, t, data)| WALEntry::new(tx, t, data))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(cases(256)))]

        #[test]
        fn wal_entry_roundtrips(entry in arb_wal_entry()) {
            let back = WALEntry::deserialize(&entry.serialize()).unwrap();
            prop_assert_eq!(back.transaction_id, entry.transaction_id);
            prop_assert_eq!(back.entry_type, entry.entry_type);
            prop_assert_eq!(&back.data, &entry.data);
            prop_assert_eq!(back.checksum, entry.checksum);
        }

        /// Any one damaged byte or a truncation is detected (CRC32 catches
        /// every error within 32 bits); never a panic
        #[test]
        fn damaged_wal_entry_is_rejected(
            entry in arb_wal_entry(),
            at in any::<prop::sample::Index>(),
            how in any::<u8>(),
        ) {
            let bytes = entry.serialize();
            let damaged = damage(bytes.clone(), at, how);
            prop_assume!(damaged != bytes);
            prop_assert!(WALEntry::deserialize(&damaged).is_err());
        }
    }

    // ------------------------------------------------------------------
    // B+ tree index files
    // ------------------------------------------------------------------

    fn arb_key() -> impl Strategy<Value = IndexKey> {
        prop_oneof![
            Just(IndexKey::Null),
            any::<bool>().prop_map(IndexKey::Bool),
            (-50i64..50).prop_map(IndexKey::Int),
            any::<i64>().prop_map(IndexKey::Int),
            arb_float().prop_map(|f| IndexKey::Float(OrderedFloat(f))),
            "[a-cő]{0,3}".prop_map(IndexKey::String),
        ]
    }

    /// (key, doc id) pairs, ids unique; keys repeat
    fn arb_entries() -> impl Strategy<Value = Vec<(IndexKey, DocumentId)>> {
        prop::collection::vec(arb_key(), 0..600).prop_map(|keys| {
            keys.into_iter()
                .enumerate()
                .map(|(i, k)| (k, DocumentId::Int(i as i64)))
                .collect()
        })
    }

    fn all_pairs(tree: &BPlusTree) -> Vec<(IndexKey, DocumentId)> {
        tree.range_query_pairs(
            &IndexKey::Null,
            &IndexKey::MaxKey,
            true,
            true,
            0,
            None,
            ScanOrder::Asc,
        )
    }

    fn saved(entries: &[(IndexKey, DocumentId)]) -> (BPlusTree, tempfile::NamedTempFile) {
        let mut tree = BPlusTree::new("i".into(), "k".into(), false, false);
        for (key, id) in entries {
            tree.insert(key.clone(), id.clone()).unwrap();
        }
        let file = tempfile::NamedTempFile::new().unwrap();
        tree.save_to_file(&mut file.reopen().unwrap()).unwrap();
        (tree, file)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(cases(48)))]

        /// Saved and loaded (eagerly and lazily), a tree holds the same
        /// pairs in the same order and counts them the same
        #[test]
        fn btree_file_roundtrips(entries in arb_entries()) {
            let (tree, file) = saved(&entries);
            let pairs = all_pairs(&tree);
            prop_assert_eq!(pairs.len(), entries.len());

            let eager = BPlusTree::load_from_file(&mut file.reopen().unwrap(), tree.metadata.clone()).unwrap();
            prop_assert_eq!(&all_pairs(&eager), &pairs);
            let lazy = BPlusTree::load_from_path_lazy(file.path().to_path_buf(), tree.metadata.clone()).unwrap();
            prop_assert_eq!(&all_pairs(&lazy), &pairs);

            for (key, _) in entries.iter().take(8) {
                let count = |t: &BPlusTree| t.range_query(key, key, true, true, RangeQueryMode::Count).unwrap_count();
                prop_assert_eq!(count(&eager), count(&tree));
                prop_assert_eq!(count(&lazy), count(&tree));
            }
        }

        /// A damaged index file loads with an error or as some tree; reading
        /// that tree does not panic either
        #[test]
        fn damaged_btree_file_never_panics(
            entries in arb_entries(),
            at in any::<prop::sample::Index>(),
            how in any::<u8>(),
        ) {
            let (tree, file) = saved(&entries);
            let mut bytes = Vec::new();
            file.reopen().unwrap().read_to_end(&mut bytes).unwrap();
            let damaged = damage(bytes, at, how);
            let mut out = file.reopen().unwrap();
            out.set_len(0).unwrap();
            out.seek(SeekFrom::Start(0)).unwrap();
            out.write_all(&damaged).unwrap();

            if let Ok(loaded) = BPlusTree::load_from_file(&mut file.reopen().unwrap(), tree.metadata.clone()) {
                let _ = all_pairs(&loaded);
            }
            if let Ok(loaded) = BPlusTree::load_from_path_lazy(file.path().to_path_buf(), tree.metadata.clone()) {
                let _ = all_pairs(&loaded);
            }
        }
    }

    // ------------------------------------------------------------------
    // HNSW index bytes
    // ------------------------------------------------------------------

    const DIM: usize = 4;

    fn arb_vectors() -> impl Strategy<Value = Vec<Vec<f32>>> {
        prop::collection::vec(prop::collection::vec(-1.0f32..1.0, DIM), 1..40)
    }

    fn built(vectors: &[Vec<f32>], removed: usize) -> HnswIndex {
        let mut index = HnswIndex::new(VectorIndexConfig {
            dim: DIM,
            ..Default::default()
        });
        for (i, v) in vectors.iter().enumerate() {
            index.insert(&i.to_string(), v).unwrap();
        }
        for i in 0..removed.min(vectors.len()) {
            index.remove(&(i * 3 % vectors.len()).to_string());
        }
        index
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(cases(48)))]

        /// Serialized and parsed back, an index (with removed vectors)
        /// answers every search exactly as before
        #[test]
        fn hnsw_bytes_roundtrip(vectors in arb_vectors(), removed in 0usize..5) {
            let index = built(&vectors, removed);
            let back = HnswIndex::from_bytes(&index.to_bytes().unwrap()).unwrap();
            prop_assert_eq!(back.len(), index.len());
            for query in vectors.iter().take(5) {
                let ids = |i: &HnswIndex| {
                    i.search(query, 5).into_iter().map(|r| (r.id, r.score.to_bits())).collect::<Vec<_>>()
                };
                prop_assert_eq!(ids(&back), ids(&index));
            }
        }

        #[test]
        fn damaged_hnsw_bytes_never_panic(
            vectors in arb_vectors(),
            at in any::<prop::sample::Index>(),
            how in any::<u8>(),
        ) {
            let bytes = built(&vectors, 1).to_bytes().unwrap();
            if let Ok(index) = HnswIndex::from_bytes(&damage(bytes, at, how)) {
                let _ = index.search(&vectors[0], 5);
                let _ = index.len();
            }
        }
    }

    // ------------------------------------------------------------------
    // Fulltext posting lists (V4)
    // ------------------------------------------------------------------

    fn arb_doc_id() -> impl Strategy<Value = DocumentId> {
        prop_oneof![
            any::<i64>().prop_map(DocumentId::Int),
            (-5i64..5).prop_map(DocumentId::Int),
            any::<String>().prop_map(DocumentId::String),
            "[0-9a-f-]{0,36}".prop_map(DocumentId::ObjectId),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(cases(256)))]

        #[test]
        fn posting_list_roundtrips(entries in prop::collection::vec((arb_doc_id(), any::<u32>()), 0..20)) {
            let back = decode_token_entries(&encode_token_entries(&entries), FTIDX_VERSION_V4).unwrap();
            prop_assert_eq!(back, entries);
        }

        #[test]
        fn damaged_posting_list_never_panics(
            entries in prop::collection::vec((arb_doc_id(), any::<u32>()), 0..20),
            at in any::<prop::sample::Index>(),
            how in any::<u8>(),
        ) {
            let _ = decode_token_entries(&damage(encode_token_entries(&entries), at, how), FTIDX_VERSION_V4);
        }

        #[test]
        fn arbitrary_bytes_as_posting_list_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..64)) {
            let _ = decode_token_entries(&bytes, FTIDX_VERSION_V4);
        }
    }
}
