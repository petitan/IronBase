//! Storing a chunked document: one document per chunk, imported idempotently
//! per `doc_id` (issue #67). Shared by the MCP `rag_document_import`,
//! `embed_document`, the Rhai `db_rag_import` and the Python `rag_import`.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

use super::Chunk;
use crate::database::DatabaseCore;
use crate::error::{IronBaseError, Result};
use crate::find_options::FindOptions;
use crate::storage::{MemoryStorage, StorageEngine};

/// Chunk-tracking fields that user-supplied metadata must never overwrite
/// (security + idempotency: `doc_id` drives replace, `chunk_index`/`*_char` the layout).
pub const RESERVED_METADATA_KEYS: &[&str] = &[
    "_id",
    "doc_id",
    "chunk_index",
    "chunk_total",
    "start_char",
    "end_char",
    "table_header",
];

/// A new `doc_id` for an imported document (UUID v4)
pub fn new_doc_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// What the chunk documents of one source document hold besides the chunks
#[derive(Debug, Clone, Copy)]
pub struct ChunkDocumentOptions<'a> {
    /// The source document's id, shared by its chunks (`doc_id`)
    pub doc_id: &'a str,
    /// Field holding the chunk text
    pub text_field: &'a str,
    /// Field holding the chunk embedding
    pub embedding_field: &'a str,
    /// `title` of every chunk
    pub title: Option<&'a str>,
    /// Document-level fields copied into every chunk; the reserved keys
    /// ([`RESERVED_METADATA_KEYS`]) and the text and embedding fields are skipped
    pub metadata: Option<&'a Map<String, Value>>,
}

/// One document per chunk: `doc_id`, `chunk_index`, `chunk_total`,
/// `start_char`, `end_char`, the text (and the embedding, when given), then
/// `title`, the chunk's heading (`section`, `heading_level`, `section_path`),
/// `table_header` and the metadata.
///
/// `embeddings`, when given, has one vector per chunk.
pub fn build_chunk_documents(
    chunks: &[Chunk],
    embeddings: Option<&[Vec<f32>]>,
    options: &ChunkDocumentOptions<'_>,
) -> Result<Vec<Value>> {
    if let Some(embeddings) = embeddings {
        if embeddings.len() != chunks.len() {
            return Err(IronBaseError::InvalidQuery(format!(
                "{} embeddings for {} chunks",
                embeddings.len(),
                chunks.len()
            )));
        }
    }
    let mut documents: Vec<Value> = Vec::new();
    documents.try_reserve(chunks.len()).map_err(|e| {
        IronBaseError::OutOfMemory(format!(
            "cannot allocate {} chunk documents: {}",
            chunks.len(),
            e
        ))
    })?;
    for (i, chunk) in chunks.iter().enumerate() {
        let mut doc = Map::new();
        doc.insert("doc_id".to_string(), json!(options.doc_id));
        doc.insert("chunk_index".to_string(), json!(chunk.index));
        doc.insert("chunk_total".to_string(), json!(chunk.total));
        doc.insert("start_char".to_string(), json!(chunk.start_char));
        doc.insert("end_char".to_string(), json!(chunk.end_char));
        doc.insert(options.text_field.to_string(), json!(chunk.text));
        if let Some(embeddings) = embeddings {
            doc.insert(options.embedding_field.to_string(), json!(embeddings[i]));
        }
        if let Some(title) = options.title {
            doc.insert("title".to_string(), json!(title));
        }
        if let Some(ref heading) = chunk.heading {
            doc.insert("section".to_string(), json!(heading));
        }
        if let Some(level) = chunk.heading_level {
            doc.insert("heading_level".to_string(), json!(level));
        }
        if let Some(ref path) = chunk.section_path {
            doc.insert("section_path".to_string(), json!(path));
        }
        if let Some(ref th) = chunk.table_header {
            doc.insert("table_header".to_string(), json!(th));
        }
        if let Some(metadata) = options.metadata {
            for (k, v) in metadata {
                // SECURITY: never let metadata overwrite the chunk-tracking
                // fields (`doc_id` drives replace) or the text / embedding
                if RESERVED_METADATA_KEYS.contains(&k.as_str())
                    || k == options.text_field
                    || k == options.embedding_field
                {
                    tracing::warn!("Ignoring protected field '{}' in chunk import metadata", k);
                    continue;
                }
                doc.insert(k.clone(), v.clone());
            }
        }
        documents.push(Value::Object(doc));
    }
    Ok(documents)
}

/// Policy for an existing `doc_id` when importing chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IfExists {
    /// Delete the previously-existing chunks for this doc_id, keep the new set (default).
    #[default]
    Replace,
    /// If chunks already exist for this doc_id, do nothing.
    Skip,
    /// If chunks already exist for this doc_id, return an error.
    Error,
    /// Always insert, leaving any existing chunks in place (legacy behavior).
    Append,
}

impl IfExists {
    /// Parse from the `if_exists` parameter. Unknown/missing → `Replace` (idempotent default).
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "skip" => Self::Skip,
            "error" => Self::Error,
            "append" => Self::Append,
            _ => Self::Replace,
        }
    }
}

/// Where chunk documents are stored: the database, or a layer over it that
/// keeps its own bookkeeping (the MCP adapter's document counts).
pub trait ChunkStore {
    /// The store's error; the import's own refusal is an [`IronBaseError`]
    type Error: From<IronBaseError>;

    /// `_id`s of the chunks stored for `doc_id` (none for a missing collection)
    fn chunk_ids(
        &self,
        collection: &str,
        doc_id: &str,
    ) -> std::result::Result<Vec<Value>, Self::Error>;

    /// Insert the documents, returning their `_id`s
    fn insert_chunk_documents(
        &self,
        collection: &str,
        documents: Vec<Value>,
    ) -> std::result::Result<Vec<Value>, Self::Error>;

    /// Delete the documents with these `_id`s, returning how many were deleted
    fn delete_chunk_documents(
        &self,
        collection: &str,
        ids: Vec<Value>,
    ) -> std::result::Result<u64, Self::Error>;
}

/// Outcome of [`import_chunks`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkImport {
    /// `_id`s of the newly inserted chunks (none when skipped)
    pub inserted_ids: Vec<Value>,
    /// Previously-existing chunks removed (Replace only)
    pub replaced: u64,
    /// The Skip policy short-circuited because chunks already existed
    pub skipped: bool,
}

fn already_exists(doc_id: &str, chunks: Option<usize>) -> IronBaseError {
    let found = chunks
        .map(|n| format!(" ({} chunks)", n))
        .unwrap_or_default();
    IronBaseError::OperationNotAllowed(format!(
        "doc_id '{}' already exists{}. Use if_exists=replace to overwrite, \
         or if_exists=append to add alongside.",
        doc_id, found
    ))
}

/// Cheap pre-check to run BEFORE the expensive embedding step.
///
/// For `Skip`/`Error` policies, detects whether `doc_id` already has chunks so the
/// caller can short-circuit without paying for embeddings it would then discard.
/// - `Replace`/`Append` → `Ok(false)` (always proceed; work happens in [`import_chunks`]).
/// - `Skip` + exists → `Ok(true)` (stop, nothing to do).
/// - `Error` + exists → `Err`.
///
/// A missing collection counts as "does not exist".
pub fn should_skip_before_embedding<S: ChunkStore + ?Sized>(
    store: &S,
    collection: &str,
    doc_id: &str,
    if_exists: IfExists,
) -> std::result::Result<bool, S::Error> {
    if matches!(if_exists, IfExists::Replace | IfExists::Append) {
        return Ok(false);
    }
    if store.chunk_ids(collection, doc_id)?.is_empty() {
        return Ok(false);
    }
    match if_exists {
        IfExists::Error => Err(already_exists(doc_id, None).into()),
        _ => Ok(true), // Skip
    }
}

/// Insert chunk documents for `doc_id` with an idempotency policy.
///
/// Safe ordering (issue #67): the previously-existing chunk ids are captured
/// first (projection: `_id` only, so embeddings are never loaded), then the new
/// chunks are inserted, and only afterwards are the old chunks deleted. A failure
/// during insert leaves the old chunks intact (no data loss); a failure during
/// the final delete leaves a temporary duplicate that the next import self-heals.
///
/// `documents` must be non-empty (callers guard the empty-chunk case earlier).
pub fn import_chunks<S: ChunkStore + ?Sized>(
    store: &S,
    collection: &str,
    doc_id: &str,
    documents: Vec<Value>,
    if_exists: IfExists,
) -> std::result::Result<ChunkImport, S::Error> {
    // Capture existing chunk ids for this doc_id (skip the lookup for Append).
    let existing_ids = if if_exists == IfExists::Append {
        Vec::new()
    } else {
        store.chunk_ids(collection, doc_id)?
    };

    if !existing_ids.is_empty() {
        match if_exists {
            IfExists::Error => {
                return Err(already_exists(doc_id, Some(existing_ids.len())).into());
            }
            IfExists::Skip => {
                return Ok(ChunkImport {
                    inserted_ids: Vec::new(),
                    replaced: 0,
                    skipped: true,
                });
            }
            _ => {}
        }
    }

    // Insert the new chunks first — old chunks stay intact if this fails.
    let inserted_ids = store.insert_chunk_documents(collection, documents)?;

    // Replace: remove the previously-existing chunks now that the new set is in.
    let replaced = if if_exists == IfExists::Replace && !existing_ids.is_empty() {
        store.delete_chunk_documents(collection, existing_ids)?
    } else {
        0
    };

    Ok(ChunkImport {
        inserted_ids,
        replaced,
        skipped: false,
    })
}

/// `_id`s of `doc_id`'s chunks via a `_id`-only projection (embeddings are
/// never loaded); none for a missing collection
fn stored_chunk_ids<S>(db: &DatabaseCore<S>, collection: &str, doc_id: &str) -> Result<Vec<Value>>
where
    S: crate::storage::Storage + crate::storage::RawStorage,
{
    let coll = match db.get_collection(collection) {
        Ok(coll) => coll,
        Err(IronBaseError::CollectionNotFound(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let options = FindOptions {
        projection: Some(HashMap::from([("_id".to_string(), 1)])),
        ..Default::default()
    };
    Ok(coll
        .find_with_options(&json!({ "doc_id": doc_id }), options)?
        .into_iter()
        .filter_map(|mut d| d.get_mut("_id").map(std::mem::take))
        .collect())
}

fn to_field_maps(documents: Vec<Value>) -> Result<Vec<HashMap<String, Value>>> {
    documents
        .into_iter()
        .map(|doc| match doc {
            Value::Object(map) => Ok(map.into_iter().collect()),
            _ => Err(IronBaseError::InvalidQuery(
                "a chunk document must be an object".to_string(),
            )),
        })
        .collect()
}

macro_rules! database_chunk_store {
    ($storage:ty) => {
        impl ChunkStore for DatabaseCore<$storage> {
            type Error = IronBaseError;

            fn chunk_ids(&self, collection: &str, doc_id: &str) -> Result<Vec<Value>> {
                stored_chunk_ids(self, collection, doc_id)
            }

            fn insert_chunk_documents(
                &self,
                collection: &str,
                documents: Vec<Value>,
            ) -> Result<Vec<Value>> {
                self.insert_many(collection, to_field_maps(documents)?)?
                    .into_iter()
                    .map(|id| Ok(serde_json::to_value(id)?))
                    .collect()
            }

            fn delete_chunk_documents(&self, collection: &str, ids: Vec<Value>) -> Result<u64> {
                self.delete_many(collection, &json!({ "_id": { "$in": ids } }))
            }
        }
    };
}

database_chunk_store!(StorageEngine);
database_chunk_store!(MemoryStorage);

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> DatabaseCore<MemoryStorage> {
        DatabaseCore::<MemoryStorage>::open_memory().unwrap()
    }

    fn chunks_of(doc_id: &str, n: usize, prefix: &str) -> Vec<Value> {
        (0..n)
            .map(|i| json!({ "doc_id": doc_id, "chunk_index": i, "content": format!("{prefix}{i}") }))
            .collect()
    }

    fn seed(db: &DatabaseCore<MemoryStorage>, doc_id: &str, n: usize) {
        db.insert_chunk_documents("rdocs", chunks_of(doc_id, n, "c"))
            .unwrap();
    }

    fn count(db: &DatabaseCore<MemoryStorage>, doc_id: &str) -> u64 {
        db.get_collection("rdocs")
            .unwrap()
            .count_documents(&json!({ "doc_id": doc_id }))
            .unwrap()
    }

    #[test]
    fn test_if_exists_parse_default_replace() {
        assert_eq!(IfExists::parse("replace"), IfExists::Replace);
        assert_eq!(IfExists::parse("SKIP"), IfExists::Skip);
        assert_eq!(IfExists::parse("error"), IfExists::Error);
        assert_eq!(IfExists::parse("append"), IfExists::Append);
        assert_eq!(IfExists::parse("nonsense"), IfExists::Replace);
        assert_eq!(IfExists::parse(""), IfExists::Replace);
    }

    #[test]
    fn test_replace_is_idempotent() {
        let db = db();
        seed(&db, "A", 3);
        // Re-import the same doc_id with a fresh 2-chunk set.
        let out = import_chunks(
            &db,
            "rdocs",
            "A",
            chunks_of("A", 2, "new"),
            IfExists::Replace,
        )
        .unwrap();
        assert_eq!(out.inserted_ids.len(), 2);
        assert_eq!(out.replaced, 3);
        assert!(!out.skipped);
        // Only the new set remains — no duplication (issue #67).
        assert_eq!(count(&db, "A"), 2);
    }

    #[test]
    fn test_replace_on_fresh_doc_id_just_inserts() {
        let db = db();
        let out = import_chunks(
            &db,
            "rdocs",
            "B",
            chunks_of("B", 4, "new"),
            IfExists::Replace,
        )
        .unwrap();
        assert_eq!(out.inserted_ids.len(), 4);
        assert_eq!(out.replaced, 0);
        assert_eq!(count(&db, "B"), 4);
    }

    #[test]
    fn test_skip_leaves_existing_untouched() {
        let db = db();
        seed(&db, "A", 3);
        let out =
            import_chunks(&db, "rdocs", "A", chunks_of("A", 2, "new"), IfExists::Skip).unwrap();
        assert!(out.skipped);
        assert_eq!(out.inserted_ids.len(), 0);
        assert_eq!(count(&db, "A"), 3);
    }

    #[test]
    fn test_error_when_exists() {
        let db = db();
        seed(&db, "A", 3);
        let res = import_chunks(&db, "rdocs", "A", chunks_of("A", 2, "new"), IfExists::Error);
        assert!(matches!(res, Err(IronBaseError::OperationNotAllowed(_))));
        assert_eq!(count(&db, "A"), 3);
    }

    #[test]
    fn test_should_skip_before_embedding() {
        let db = db();
        seed(&db, "A", 3);

        // Replace/Append always proceed (never skip), even when chunks exist.
        assert!(!should_skip_before_embedding(&db, "rdocs", "A", IfExists::Replace).unwrap());
        assert!(!should_skip_before_embedding(&db, "rdocs", "A", IfExists::Append).unwrap());

        // Skip: existing → true (stop), non-existing → false (proceed).
        assert!(should_skip_before_embedding(&db, "rdocs", "A", IfExists::Skip).unwrap());
        assert!(!should_skip_before_embedding(&db, "rdocs", "Z", IfExists::Skip).unwrap());

        // Error: existing → Err, non-existing → Ok(false).
        assert!(should_skip_before_embedding(&db, "rdocs", "A", IfExists::Error).is_err());
        assert!(!should_skip_before_embedding(&db, "rdocs", "Z", IfExists::Error).unwrap());

        // Missing collection counts as "does not exist".
        assert!(!should_skip_before_embedding(&db, "nope", "A", IfExists::Skip).unwrap());
    }

    #[test]
    fn test_append_adds_alongside() {
        let db = db();
        seed(&db, "A", 3);
        let out = import_chunks(
            &db,
            "rdocs",
            "A",
            chunks_of("A", 2, "new"),
            IfExists::Append,
        )
        .unwrap();
        assert_eq!(out.inserted_ids.len(), 2);
        assert_eq!(out.replaced, 0);
        // Legacy behavior: old + new coexist.
        assert_eq!(count(&db, "A"), 5);
    }

    fn chunk(index: usize, text: &str, heading: Option<&str>) -> Chunk {
        Chunk {
            index,
            total: 2,
            text: text.to_string(),
            start_char: index * 10,
            end_char: index * 10 + text.len(),
            heading: heading.map(str::to_string),
            heading_level: heading.map(|_| 2),
            section_path: heading.map(|h| vec![h.to_string()]),
            table_header: None,
        }
    }

    #[test]
    fn chunk_documents_carry_the_layout_and_filtered_metadata() {
        let chunks = [chunk(0, "alpha", Some("Intro")), chunk(1, "beta", None)];
        let metadata = json!({"year": 2026, "doc_id": "evil", "content": "evil", "vec": [1]});
        let options = ChunkDocumentOptions {
            doc_id: "D",
            text_field: "content",
            embedding_field: "vec",
            title: Some("T"),
            metadata: metadata.as_object(),
        };
        let docs =
            build_chunk_documents(&chunks, Some(&[vec![1.0, 0.0], vec![0.0, 1.0]]), &options)
                .unwrap();
        assert_eq!(
            docs[0],
            json!({"doc_id": "D", "chunk_index": 0, "chunk_total": 2, "start_char": 0,
                "end_char": 5, "content": "alpha", "vec": [1.0, 0.0], "title": "T",
                "section": "Intro", "heading_level": 2, "section_path": ["Intro"],
                "year": 2026})
        );
        assert_eq!(docs[1]["vec"], json!([0.0, 1.0]));
        assert!(docs[1].get("section").is_none());

        // Without embeddings: no vector field
        let docs = build_chunk_documents(&chunks, None, &options).unwrap();
        assert!(docs[0].get("vec").is_none());
        // One vector per chunk
        assert!(build_chunk_documents(&chunks, Some(&[vec![1.0]]), &options).is_err());
    }
}
