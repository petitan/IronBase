// catalog_serde.rs
// Custom serialization for HashMap<DocumentId, u64> to preserve DocumentId types in JSON

use crate::document::DocumentId;
use serde::de::{SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserializer, Serializer};
use std::collections::HashMap;

/// Serialize HashMap<DocumentId, u64> as array of [type_tag, value, offset] tuples
/// This preserves DocumentId type information in JSON metadata
pub fn serialize<S>(catalog: &HashMap<DocumentId, u64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(catalog.len()))?;
    for (doc_id, offset) in catalog {
        // Serialize as [type_tag, value, offset]
        // type_tag: "i" = Int, "s" = String, "o" = ObjectId
        let entry: (&str, String, u64) = match doc_id {
            DocumentId::Int(i) => ("i", i.to_string(), *offset),
            DocumentId::String(s) => ("s", s.clone(), *offset),
            DocumentId::ObjectId(oid) => ("o", oid.clone(), *offset),
        };
        seq.serialize_element(&entry)?;
    }
    seq.end()
}

/// Deserialize array of [type_tag, value, offset] tuples back to HashMap<DocumentId, u64>
pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<DocumentId, u64>, D::Error>
where
    D: Deserializer<'de>,
{
    struct CatalogVisitor;

    impl<'de> Visitor<'de> for CatalogVisitor {
        type Value = HashMap<DocumentId, u64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an array of [type_tag, value, offset] tuples")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut catalog = HashMap::new();

            while let Some((type_tag, value_str, offset)) =
                seq.next_element::<(String, String, u64)>()?
            {
                let doc_id = match type_tag.as_str() {
                    "i" => {
                        let val = value_str.parse::<i64>().map_err(|e| {
                            serde::de::Error::custom(format!("Invalid Int value: {}", e))
                        })?;
                        DocumentId::Int(val)
                    }
                    "s" => DocumentId::String(value_str),
                    "o" => DocumentId::ObjectId(value_str),
                    _ => {
                        return Err(serde::de::Error::custom(format!(
                            "Unknown type tag: {}",
                            type_tag
                        )))
                    }
                };
                catalog.insert(doc_id, offset);
            }

            Ok(catalog)
        }
    }

    deserializer.deserialize_seq(CatalogVisitor)
}

/// `Vec<(DocumentId, T)>` as `[type_tag, value, T]` triples; reads the
/// untagged `[id, T]` pairs of older files too. Untagged JSON turns an
/// ObjectId into a String on reload, so per-document data keyed by it (e.g.
/// the fulltext index's BM25 document lengths) was no longer found
/// (hybrid review D2).
pub mod pairs {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Entry<T> {
        Typed(String, String, T),
        Legacy(DocumentId, T),
    }

    pub fn serialize<S, T>(pairs: &[(DocumentId, T)], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize,
    {
        let mut seq = serializer.serialize_seq(Some(pairs.len()))?;
        for (doc_id, value) in pairs {
            let (tag, id) = match doc_id {
                DocumentId::Int(i) => ("i", i.to_string()),
                DocumentId::String(s) => ("s", s.clone()),
                DocumentId::ObjectId(oid) => ("o", oid.clone()),
            };
            seq.serialize_element(&(tag, id, value))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<Vec<(DocumentId, T)>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        Vec::<Entry<T>>::deserialize(deserializer)?
            .into_iter()
            .map(|entry| match entry {
                Entry::Legacy(doc_id, value) => Ok((doc_id, value)),
                Entry::Typed(tag, id, value) => {
                    let doc_id = match tag.as_str() {
                        "i" => DocumentId::Int(id.parse().map_err(|e| {
                            serde::de::Error::custom(format!("Invalid Int value: {}", e))
                        })?),
                        "s" => DocumentId::String(id),
                        "o" => DocumentId::ObjectId(id),
                        _ => {
                            return Err(serde::de::Error::custom(format!(
                                "Unknown type tag: {}",
                                tag
                            )))
                        }
                    };
                    Ok((doc_id, value))
                }
            })
            .collect()
    }
}

pub mod vec {
    use super::*;
    use serde::de::{SeqAccess, Visitor};
    use serde::ser::SerializeSeq;

    /// Serialize Vec<DocumentId> as array of [type_tag, value] tuples
    pub fn serialize<S>(ids: &Vec<DocumentId>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(ids.len()))?;
        for doc_id in ids {
            let entry: (&str, String) = match doc_id {
                DocumentId::Int(i) => ("i", i.to_string()),
                DocumentId::String(s) => ("s", s.clone()),
                DocumentId::ObjectId(oid) => ("o", oid.clone()),
            };
            seq.serialize_element(&entry)?;
        }
        seq.end()
    }

    /// Deserialize array of [type_tag, value] tuples back to Vec<DocumentId>
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<DocumentId>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IdVecVisitor;

        impl<'de> Visitor<'de> for IdVecVisitor {
            type Value = Vec<DocumentId>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an array of [type_tag, value] tuples")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut ids = Vec::new();
                while let Some((type_tag, value_str)) = seq.next_element::<(String, String)>()? {
                    let doc_id = match type_tag.as_str() {
                        "i" => {
                            let val = value_str.parse::<i64>().map_err(|e| {
                                serde::de::Error::custom(format!("Invalid Int value: {}", e))
                            })?;
                            DocumentId::Int(val)
                        }
                        "s" => DocumentId::String(value_str),
                        "o" => DocumentId::ObjectId(value_str),
                        _ => {
                            return Err(serde::de::Error::custom(format!(
                                "Unknown type tag: {}",
                                type_tag
                            )))
                        }
                    };
                    ids.push(doc_id);
                }
                Ok(ids)
            }
        }

        deserializer.deserialize_seq(IdVecVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// Wrapper struct to test the custom serialization
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestCatalog {
        #[serde(serialize_with = "serialize", deserialize_with = "deserialize")]
        catalog: HashMap<DocumentId, u64>,
    }

    #[test]
    fn test_roundtrip_empty_catalog() {
        let original = TestCatalog {
            catalog: HashMap::new(),
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
        assert!(restored.catalog.is_empty());
    }

    #[test]
    fn test_roundtrip_int_document_id() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(42), 1000u64);
        catalog.insert(DocumentId::Int(0), 2000u64);
        catalog.insert(DocumentId::Int(-123), 3000u64);
        catalog.insert(DocumentId::Int(i64::MAX), 4000u64);
        catalog.insert(DocumentId::Int(i64::MIN), 5000u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_roundtrip_string_document_id() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::String("hello".to_string()), 100u64);
        catalog.insert(DocumentId::String("".to_string()), 200u64);
        catalog.insert(DocumentId::String("with spaces".to_string()), 300u64);
        catalog.insert(DocumentId::String("special!@#$%".to_string()), 400u64);
        catalog.insert(DocumentId::String("unicode_ű_字".to_string()), 500u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_roundtrip_objectid_document_id() {
        let mut catalog = HashMap::new();
        catalog.insert(
            DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string()),
            100u64,
        );
        catalog.insert(
            DocumentId::ObjectId("000000000000000000000000".to_string()),
            200u64,
        );
        catalog.insert(
            DocumentId::ObjectId("ffffffffffffffffffffffff".to_string()),
            300u64,
        );

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_roundtrip_mixed_document_ids() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(1), 100u64);
        catalog.insert(DocumentId::String("doc-2".to_string()), 200u64);
        catalog.insert(
            DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string()),
            300u64,
        );
        catalog.insert(DocumentId::Int(-999), 400u64);
        catalog.insert(DocumentId::String("another".to_string()), 500u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_roundtrip_large_offsets() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(1), 0u64);
        catalog.insert(DocumentId::Int(2), u64::MAX);
        catalog.insert(DocumentId::Int(3), u64::MAX / 2);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_serialize_format() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(42), 1000u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();

        // Check that the JSON contains the type tag "i" for Int
        assert!(json.contains("\"i\""));
        assert!(json.contains("\"42\""));
        assert!(json.contains("1000"));
    }

    #[test]
    fn test_serialize_string_format() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::String("my-id".to_string()), 2000u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();

        // Check that the JSON contains the type tag "s" for String
        assert!(json.contains("\"s\""));
        assert!(json.contains("\"my-id\""));
    }

    #[test]
    fn test_serialize_objectid_format() {
        let mut catalog = HashMap::new();
        catalog.insert(
            DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string()),
            3000u64,
        );

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();

        // Check that the JSON contains the type tag "o" for ObjectId
        assert!(json.contains("\"o\""));
        assert!(json.contains("507f1f77bcf86cd799439011"));
    }

    #[test]
    fn test_deserialize_invalid_type_tag() {
        // JSON with unknown type tag "x"
        let json = r#"{"catalog":[["x","value",1000]]}"#;
        let result: Result<TestCatalog, _> = serde_json::from_str(json);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown type tag"));
    }

    #[test]
    fn test_deserialize_invalid_int_value() {
        // JSON with type tag "i" (Int) but non-numeric value
        let json = r#"{"catalog":[["i","not_a_number",1000]]}"#;
        let result: Result<TestCatalog, _> = serde_json::from_str(json);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid Int value"));
    }

    #[test]
    fn test_deserialize_int_overflow() {
        // JSON with type tag "i" but value too large for i64
        let json = r#"{"catalog":[["i","99999999999999999999999999999",1000]]}"#;
        let result: Result<TestCatalog, _> = serde_json::from_str(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip_many_entries() {
        let mut catalog = HashMap::new();
        for i in 0..1000 {
            catalog.insert(DocumentId::Int(i), i as u64 * 100);
        }

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
        assert_eq!(restored.catalog.len(), 1000);
    }

    #[test]
    fn test_deserialize_malformed_tuple() {
        // JSON with wrong tuple structure (only 2 elements instead of 3)
        let json = r#"{"catalog":[["i","42"]]}"#;
        let result: Result<TestCatalog, _> = serde_json::from_str(json);

        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_type_tag() {
        // JSON with empty type tag
        let json = r#"{"catalog":[["","value",1000]]}"#;
        let result: Result<TestCatalog, _> = serde_json::from_str(json);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown type tag"));
    }

    #[test]
    fn test_string_that_looks_like_int() {
        // String document ID that looks like a number - should remain String type
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::String("12345".to_string()), 100u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
        // Verify it's still a String type, not converted to Int
        assert!(restored
            .catalog
            .contains_key(&DocumentId::String("12345".to_string())));
        assert!(!restored.catalog.contains_key(&DocumentId::Int(12345)));
    }

    #[test]
    fn test_objectid_that_looks_like_string() {
        // ObjectId should stay ObjectId, not become String
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::ObjectId("abc123".to_string()), 100u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
        assert!(restored
            .catalog
            .contains_key(&DocumentId::ObjectId("abc123".to_string())));
    }

    #[test]
    fn test_negative_int_roundtrip() {
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(-1), 100u64);
        catalog.insert(DocumentId::Int(-9223372036854775808), 200u64); // i64::MIN

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn test_duplicate_offset_different_ids() {
        // Different document IDs can have the same offset (shouldn't happen in practice but test it)
        let mut catalog = HashMap::new();
        catalog.insert(DocumentId::Int(1), 1000u64);
        catalog.insert(DocumentId::Int(2), 1000u64);
        catalog.insert(DocumentId::String("three".to_string()), 1000u64);

        let original = TestCatalog { catalog };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestCatalog = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
        assert_eq!(restored.catalog.len(), 3);
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestIdVec {
        #[serde(
            serialize_with = "vec::serialize",
            deserialize_with = "vec::deserialize"
        )]
        ids: Vec<DocumentId>,
    }

    #[test]
    fn test_id_vec_roundtrip() {
        let original = TestIdVec {
            ids: vec![
                DocumentId::Int(1),
                DocumentId::String("abc".to_string()),
                DocumentId::ObjectId("507f1f77bcf86cd799439011".to_string()),
            ],
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: TestIdVec = serde_json::from_str(&json).unwrap();

        assert_eq!(original, restored);
    }

    /// D2: typed pairs keep the id kind; untagged pairs of older files load
    #[test]
    fn test_pairs_typed_roundtrip_and_legacy() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Holder {
            #[serde(with = "super::pairs")]
            pairs: Vec<(DocumentId, u32)>,
        }
        let holder = Holder {
            pairs: vec![
                (DocumentId::Int(-4), 1),
                (DocumentId::String("507f1f77bcf86cd799439011".into()), 2),
                (DocumentId::ObjectId("507f1f77bcf86cd799439011".into()), 3),
            ],
        };
        let json = serde_json::to_string(&holder).unwrap();
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), holder);

        let legacy: Holder = serde_json::from_str(r#"{"pairs":[[7,5],["abc",2]]}"#).unwrap();
        assert_eq!(
            legacy.pairs,
            vec![
                (DocumentId::Int(7), 5),
                (DocumentId::String("abc".into()), 2)
            ]
        );
        assert!(serde_json::from_str::<Holder>(r#"{"pairs":[["x","1",2]]}"#).is_err());
    }
}
