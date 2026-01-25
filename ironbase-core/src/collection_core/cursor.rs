// FindCursor - Memory-efficient iterator for large result sets

use serde_json::Value;
use std::collections::HashSet;

use crate::document::{Document, DocumentId};
use crate::error::{IronBaseError, Result};
use crate::index::{IndexKey, ScanOrder};
use crate::query::Query;
use crate::query_planner::QueryPlan;
use crate::storage::{RawStorage, Storage, HEADER_SIZE};

use super::CollectionCore;

// Re-export from central limits module
use crate::limits::DEFAULT_CURSOR_BATCH_SIZE;

/// Iterator for streaming query results
///
/// Provides memory-efficient iteration over large result sets without
/// loading all documents into memory at once.
///
/// # Example
/// ```rust,ignore
/// let mut cursor = collection.find_streaming(&query)?;
///
/// // Option 1: Process in chunks
/// while !cursor.is_finished() {
///     let batch = cursor.next_chunk(100)?;
///     for doc in batch {
///         process(doc);
///     }
/// }
///
/// // Option 2: Process one at a time
/// while let Some(doc) = cursor.next()? {
///     process(doc);
/// }
///
/// // Option 3: Process with for_each
/// cursor.for_each(|doc| {
///     process(doc);
///     Ok(())  // Return Err to stop iteration
/// })?;
/// ```
struct IndexBatchState {
    index_name: String,
    range_start: IndexKey,
    range_end: IndexKey,
    inclusive_start: bool,
    inclusive_end: bool,
    order: ScanOrder,
    current_start: IndexKey,
    current_skip: usize,
    buffer: Vec<DocumentId>,
    buffer_pos: usize,
    yielded: usize,
    exhausted: bool,
    multikey: bool,
    seen: Option<HashSet<DocumentId>>,
}

enum FindCursorMode {
    DocIds {
        doc_ids: Vec<DocumentId>,
        position: usize,
    },
    IndexScan {
        query: Query,
        state: IndexBatchState,
    },
    Scan {
        collection_name: String,
        query: Query,
        offset: u64,
        data_end: u64,
        yielded: usize,
    },
}

pub struct FindCursor<'a, S: Storage + RawStorage> {
    collection: &'a CollectionCore<S>,
    mode: FindCursorMode,
    /// Default batch size for chunk operations
    batch_size: usize,
}

impl<'a, S: Storage + RawStorage> FindCursor<'a, S> {
    /// Create a new cursor with the given document IDs
    pub(crate) fn new(collection: &'a CollectionCore<S>, doc_ids: Vec<DocumentId>) -> Self {
        FindCursor {
            collection,
            mode: FindCursorMode::DocIds {
                doc_ids,
                position: 0,
            },
            batch_size: DEFAULT_CURSOR_BATCH_SIZE,
        }
    }

    pub(crate) fn new_index_scan(
        collection: &'a CollectionCore<S>,
        query_json: &Value,
        index_name: String,
        range_start: IndexKey,
        range_end: IndexKey,
        inclusive_start: bool,
        inclusive_end: bool,
    ) -> Result<Self> {
        let query = Query::from_json(query_json)?;
        let (multikey, seen) = {
            let indexes = collection.indexes.read();
            let multikey = indexes
                .get_btree_index(&index_name)
                .map(|index| index.metadata.multikey)
                .unwrap_or(false);
            (multikey, if multikey { Some(HashSet::new()) } else { None })
        };

        Ok(FindCursor {
            collection,
            mode: FindCursorMode::IndexScan {
                query,
                state: IndexBatchState {
                    index_name,
                    range_start: range_start.clone(),
                    range_end: range_end.clone(),
                    inclusive_start,
                    inclusive_end,
                    order: ScanOrder::Asc,
                    current_start: range_start,
                    current_skip: 0,
                    buffer: Vec::new(),
                    buffer_pos: 0,
                    yielded: 0,
                    exhausted: false,
                    multikey,
                    seen,
                },
            },
            batch_size: DEFAULT_CURSOR_BATCH_SIZE,
        })
    }

    /// Create a new streaming cursor that scans storage sequentially.
    pub(crate) fn new_scan(collection: &'a CollectionCore<S>, query_json: &Value) -> Result<Self> {
        let query = Query::from_json(query_json)?;
        let storage = collection.storage.read();
        if storage.get_file_path().is_empty() {
            drop(storage);
            let (doc_ids, _) = collection.collect_doc_ids_with_options(
                query_json, None, None, false, 0, None, true, 0, None, None, None,
            )?;
            return Ok(FindCursor::new(collection, doc_ids));
        }
        let file_len = storage.file_len()?;
        let mut metadata_offset = storage.metadata_offset();
        let metadata_size = storage.metadata_size();
        let data_end_offset = storage.data_end_offset();
        if metadata_offset == 0 && metadata_size > 0 && data_end_offset >= metadata_size {
            metadata_offset = data_end_offset - metadata_size;
        }
        let needs_catalog_fallback = metadata_size == 0 && data_end_offset > HEADER_SIZE;
        if needs_catalog_fallback {
            drop(storage);
            let (doc_ids, _) = collection.collect_doc_ids_with_options(
                query_json, None, None, false, 0, None, true, 0, None, None, None,
            )?;
            return Ok(FindCursor::new(collection, doc_ids));
        }
        let mut start_offset = HEADER_SIZE;
        let mut data_end = file_len;

        if metadata_offset >= HEADER_SIZE && metadata_offset <= file_len && metadata_size > 0 {
            if metadata_offset == HEADER_SIZE && data_end_offset > metadata_offset + metadata_size {
                // Metadata is at the beginning; documents are after metadata.
                start_offset = metadata_offset + metadata_size;
                data_end = data_end_offset.min(file_len);
            } else {
                // Metadata is at EOF; documents end at metadata_offset.
                data_end = metadata_offset;
            }
        }

        if let Some(meta) = storage.get_collection_meta(&collection.name) {
            if let (Some(min_offset), Some(max_offset)) = (
                meta.document_catalog.values().min().copied(),
                meta.document_catalog.values().max().copied(),
            ) {
                if min_offset > start_offset {
                    start_offset = min_offset;
                }
                if let Ok(bytes) = storage.read_data_at(max_offset) {
                    data_end = max_offset + 4 + bytes.len() as u64;
                }
            }
        }
        drop(storage);

        Ok(FindCursor {
            collection,
            mode: FindCursorMode::Scan {
                collection_name: collection.name.clone(),
                query,
                offset: start_offset,
                data_end,
                yielded: 0,
            },
            batch_size: DEFAULT_CURSOR_BATCH_SIZE,
        })
    }

    pub(crate) fn new_index_scan_from_plan(
        collection: &'a CollectionCore<S>,
        query_json: &Value,
        plan: QueryPlan,
    ) -> Result<Option<Self>> {
        match plan {
            QueryPlan::IndexScan {
                index_name,
                key,
                is_compound,
                ..
            } => {
                let (start, end) = if is_compound {
                    let indexes = collection.indexes.read();
                    let Some(index) = indexes.get_btree_index(&index_name) else {
                        return Ok(None);
                    };
                    index.build_prefix_range(key.clone())
                } else {
                    (key.clone(), key.clone())
                };
                Ok(Some(FindCursor::new_index_scan(
                    collection, query_json, index_name, start, end, true, true,
                )?))
            }
            QueryPlan::IndexRangeScan {
                index_name,
                start,
                end,
                inclusive_start,
                inclusive_end,
                ..
            } => {
                let default_start = IndexKey::Null;
                let default_end = IndexKey::MaxKey;
                let start_key = start.unwrap_or(default_start);
                let end_key = end.unwrap_or(default_end);
                Ok(Some(FindCursor::new_index_scan(
                    collection,
                    query_json,
                    index_name,
                    start_key,
                    end_key,
                    inclusive_start,
                    inclusive_end,
                )?))
            }
            QueryPlan::RegexPrefixScan {
                index_name, prefix, ..
            } => {
                let start = IndexKey::String(prefix.clone());
                let end = IndexKey::String(format!("{}\u{10ffff}", prefix));
                Ok(Some(FindCursor::new_index_scan(
                    collection, query_json, index_name, start, end, true, true,
                )?))
            }
            QueryPlan::SparseIndexScan { index_name, .. } => Ok(Some(FindCursor::new_index_scan(
                collection,
                query_json,
                index_name,
                IndexKey::Null,
                IndexKey::MaxKey,
                true,
                true,
            )?)),
            QueryPlan::MultiRegexPrefixScan { .. } => Ok(None),
            // MultiValueScan is handled by collect_doc_ids_from_plan, not cursor
            QueryPlan::MultiValueScan { .. } => Ok(None),
        }
    }

    /// Set the default batch size for chunk operations
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size;
        self
    }

    /// Fetch the next document, or None if exhausted
    /// Uses iterative loop instead of recursion to avoid stack overflow with many tombstones
    pub fn next(&mut self) -> Result<Option<Value>> {
        let batch_size = self.batch_size;
        let collection = self.collection;

        match &mut self.mode {
            FindCursorMode::DocIds { doc_ids, position } => loop {
                if *position >= doc_ids.len() {
                    return Ok(None);
                }

                let doc_id = &doc_ids[*position];
                *position += 1;

                match self.collection.read_document_by_id(doc_id)? {
                    Some(doc) => return Ok(Some(doc)),
                    None => continue, // Skip tombstones, continue to next
                }
            },
            FindCursorMode::IndexScan { query, state } => loop {
                if state.exhausted {
                    return Ok(None);
                }
                if state.buffer_pos >= state.buffer.len() {
                    Self::fill_index_buffer(collection, batch_size, state)?;
                    if state.exhausted {
                        return Ok(None);
                    }
                    if state.buffer.is_empty() {
                        continue;
                    }
                }

                let doc_id = state.buffer[state.buffer_pos].clone();
                state.buffer_pos += 1;

                if state.multikey {
                    if let Some(seen) = state.seen.as_mut() {
                        if !seen.insert(doc_id.clone()) {
                            continue;
                        }
                    }
                }

                if let Some(doc) = self.collection.read_document_by_id(&doc_id)? {
                    let document = Document::from_value(&doc)?;
                    if query.matches(&document)? {
                        state.yielded += 1;
                        return Ok(Some(doc));
                    }
                }
            },
            FindCursorMode::Scan {
                collection_name,
                query,
                offset,
                data_end,
                yielded,
            } => loop {
                if *offset >= *data_end {
                    return Ok(None);
                }

                let current_offset = *offset;
                let data = {
                    let storage = self.collection.storage.read();
                    storage.read_data_at(current_offset)?
                };
                *offset = current_offset + 4 + data.len() as u64;

                let doc: Value = serde_json::from_slice(&data).map_err(|e| {
                    IronBaseError::Corruption(format!(
                        "Failed to parse document at offset {}: {}",
                        current_offset, e
                    ))
                })?;

                if doc
                    .get("_tombstone")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    continue;
                }

                if doc.get("_collection").and_then(|v| v.as_str()) != Some(collection_name.as_str())
                {
                    continue;
                }

                let doc_id = match doc.get("_id") {
                    Some(id_val) => match serde_json::from_value::<DocumentId>(id_val.clone()) {
                        Ok(id) => id,
                        Err(_) => continue,
                    },
                    None => continue,
                };

                let offset_matches = {
                    let storage = self.collection.storage.read();
                    let meta = storage
                        .get_collection_meta(collection_name)
                        .ok_or_else(|| {
                            IronBaseError::CollectionNotFound(collection_name.clone())
                        })?;
                    meta.document_catalog
                        .get(&doc_id)
                        .copied()
                        .unwrap_or(u64::MAX)
                        == current_offset
                };
                if !offset_matches {
                    continue;
                }

                let document = Document::from_value(&doc)?;
                if query.matches(&document)? {
                    *yielded += 1;
                    return Ok(Some(doc));
                }
            },
        }
    }

    /// Fetch the next chunk of documents (up to `chunk_size`)
    pub fn next_chunk(&mut self, chunk_size: usize) -> Result<Vec<Value>> {
        let mut results = Vec::with_capacity(chunk_size);
        for _ in 0..chunk_size {
            match self.next()? {
                Some(doc) => results.push(doc),
                None => break,
            }
        }
        Ok(results)
    }

    /// Fetch the next chunk using the default batch size
    pub fn next_batch(&mut self) -> Result<Vec<Value>> {
        self.next_chunk(self.batch_size)
    }

    /// Remaining documents in the cursor
    pub fn remaining(&self) -> usize {
        match &self.mode {
            FindCursorMode::DocIds { doc_ids, position } => doc_ids.len().saturating_sub(*position),
            FindCursorMode::IndexScan { .. } => 0,
            FindCursorMode::Scan { .. } => 0,
        }
    }

    /// Total document count
    pub fn total(&self) -> usize {
        match &self.mode {
            FindCursorMode::DocIds { doc_ids, .. } => doc_ids.len(),
            FindCursorMode::IndexScan { .. } => 0,
            FindCursorMode::Scan { .. } => 0,
        }
    }

    /// Current position in the cursor
    pub fn position(&self) -> usize {
        match &self.mode {
            FindCursorMode::DocIds { position, .. } => *position,
            FindCursorMode::IndexScan { state, .. } => state.yielded,
            FindCursorMode::Scan { yielded, .. } => *yielded,
        }
    }

    /// Check if cursor is exhausted
    pub fn is_finished(&self) -> bool {
        match &self.mode {
            FindCursorMode::DocIds { doc_ids, position } => *position >= doc_ids.len(),
            FindCursorMode::IndexScan { state, .. } => state.exhausted,
            FindCursorMode::Scan {
                offset, data_end, ..
            } => *offset >= *data_end,
        }
    }

    /// Reset cursor to the beginning
    pub fn rewind(&mut self) {
        match &mut self.mode {
            FindCursorMode::DocIds { position, .. } => {
                *position = 0;
            }
            FindCursorMode::IndexScan { state, .. } => {
                state.current_start = state.range_start.clone();
                state.current_skip = 0;
                state.buffer.clear();
                state.buffer_pos = 0;
                state.yielded = 0;
                state.exhausted = false;
                if let Some(seen) = state.seen.as_mut() {
                    seen.clear();
                }
            }
            FindCursorMode::Scan {
                offset, yielded, ..
            } => {
                *offset = HEADER_SIZE;
                *yielded = 0;
            }
        }
    }

    /// Skip the next N documents
    pub fn skip(&mut self, n: usize) {
        match &mut self.mode {
            FindCursorMode::DocIds { doc_ids, position } => {
                *position = (*position + n).min(doc_ids.len());
            }
            FindCursorMode::IndexScan { .. } => {
                for _ in 0..n {
                    if self.next().ok().flatten().is_none() {
                        break;
                    }
                }
            }
            FindCursorMode::Scan { .. } => {
                for _ in 0..n {
                    if self.next().ok().flatten().is_none() {
                        break;
                    }
                }
            }
        }
    }

    /// Process each document with a closure
    ///
    /// The closure can return Err to stop iteration early
    pub fn for_each<F>(&mut self, mut f: F) -> Result<()>
    where
        F: FnMut(Value) -> Result<()>,
    {
        while let Some(doc) = self.next()? {
            f(doc)?;
        }
        Ok(())
    }

    /// Collect all remaining documents into a Vec
    ///
    /// Warning: This loads all remaining documents into memory.
    /// Uses try_reserve() to fail gracefully on OOM instead of panic.
    pub fn collect_all(&mut self) -> Result<Vec<Value>> {
        let count = self.remaining();
        let mut results = Vec::new();
        // OOM FIX: Use try_reserve() for graceful error on allocation failure
        results.try_reserve(count).map_err(|e| {
            crate::error::IronBaseError::OutOfMemory(format!(
                "Cannot allocate {} documents for cursor.collect_all(): {}",
                count, e
            ))
        })?;
        while let Some(doc) = self.next()? {
            results.push(doc);
        }
        Ok(results)
    }

    /// Take the next N documents
    ///
    /// Uses try_reserve() to fail gracefully on OOM instead of panic.
    pub fn take(&mut self, n: usize) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        // OOM FIX: Use try_reserve() for graceful error on allocation failure
        results.try_reserve(n).map_err(|e| {
            crate::error::IronBaseError::OutOfMemory(format!(
                "Cannot allocate {} documents for cursor.take(): {}",
                n, e
            ))
        })?;
        for _ in 0..n {
            match self.next()? {
                Some(doc) => results.push(doc),
                None => break,
            }
        }
        Ok(results)
    }

    fn fill_index_buffer(
        collection: &CollectionCore<S>,
        batch_size: usize,
        state: &mut IndexBatchState,
    ) -> Result<()> {
        if state.exhausted {
            return Ok(());
        }

        let batch = {
            let indexes = collection.indexes.read();
            let Some(index) = indexes.get_btree_index(&state.index_name) else {
                state.exhausted = true;
                return Ok(());
            };
            index.range_query_pairs(
                &state.current_start,
                &state.range_end,
                state.inclusive_start,
                state.inclusive_end,
                state.current_skip,
                Some(batch_size),
                state.order,
            )
        };

        if batch.is_empty() {
            state.exhausted = true;
            return Ok(());
        }

        state.buffer.clear();
        state.buffer_pos = 0;
        for (_, doc_id) in &batch {
            state.buffer.push(doc_id.clone());
        }

        let last_key = batch.last().unwrap().0.clone();
        let mut last_key_count = 0usize;
        for (key, _) in batch.iter().rev() {
            if *key == last_key {
                last_key_count += 1;
            } else {
                break;
            }
        }

        if state.current_start == last_key {
            state.current_skip += last_key_count;
        } else {
            state.current_skip = last_key_count;
        }
        state.current_start = last_key;
        state.inclusive_start = true;

        Ok(())
    }
}
