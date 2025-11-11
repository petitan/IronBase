// catalog_serde.rs
// Custom serialization for HashMap<DocumentId, u64> to preserve DocumentId types in JSON

use std::collections::HashMap;
use serde::{Serializer, Deserializer};
use serde::ser::SerializeSeq;
use serde::de::{SeqAccess, Visitor};
use crate::document::DocumentId;

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

            while let Some((type_tag, value_str, offset)) = seq.next_element::<(String, String, u64)>()? {
                let doc_id = match type_tag.as_str() {
                    "i" => {
                        let val = value_str.parse::<i64>()
                            .map_err(|e| serde::de::Error::custom(format!("Invalid Int value: {}", e)))?;
                        DocumentId::Int(val)
                    },
                    "s" => DocumentId::String(value_str),
                    "o" => DocumentId::ObjectId(value_str),
                    _ => return Err(serde::de::Error::custom(format!("Unknown type tag: {}", type_tag))),
                };
                catalog.insert(doc_id, offset);
            }

            Ok(catalog)
        }
    }

    deserializer.deserialize_seq(CatalogVisitor)
}
