// FindCursor - Memory-efficient iterator for large result sets

use serde_json::Value;

use crate::document::DocumentId;
use crate::error::Result;
use crate::storage::{RawStorage, Storage};

use super::CollectionCore;

/// Default batch size for streaming cursor operations
const DEFAULT_CURSOR_BATCH_SIZE: usize = 100;

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
pub struct FindCursor<'a, S: Storage + RawStorage> {
    collection: &'a CollectionCore<S>,
    doc_ids: Vec<DocumentId>,
    position: usize,
    /// Default batch size for chunk operations
    batch_size: usize,
}

impl<'a, S: Storage + RawStorage> FindCursor<'a, S> {
    /// Create a new cursor with the given document IDs
    pub(crate) fn new(collection: &'a CollectionCore<S>, doc_ids: Vec<DocumentId>) -> Self {
        FindCursor {
            collection,
            doc_ids,
            position: 0,
            batch_size: DEFAULT_CURSOR_BATCH_SIZE,
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
        loop {
            if self.position >= self.doc_ids.len() {
                return Ok(None);
            }

            let doc_id = &self.doc_ids[self.position];
            self.position += 1;

            match self.collection.read_document_by_id(doc_id)? {
                Some(doc) => return Ok(Some(doc)),
                None => continue, // Skip tombstones, continue to next
            }
        }
    }

    /// Fetch the next chunk of documents (up to `chunk_size`)
    pub fn next_chunk(&mut self, chunk_size: usize) -> Result<Vec<Value>> {
        if self.position >= self.doc_ids.len() {
            return Ok(Vec::new());
        }

        let end = (self.position + chunk_size).min(self.doc_ids.len());
        let mut results = Vec::with_capacity(end - self.position);
        for doc_id in &self.doc_ids[self.position..end] {
            if let Some(doc) = self.collection.read_document_by_id(doc_id)? {
                results.push(doc);
            }
        }
        self.position = end;
        Ok(results)
    }

    /// Fetch the next chunk using the default batch size
    pub fn next_batch(&mut self) -> Result<Vec<Value>> {
        self.next_chunk(self.batch_size)
    }

    /// Remaining documents in the cursor
    pub fn remaining(&self) -> usize {
        self.doc_ids.len().saturating_sub(self.position)
    }

    /// Total document count
    pub fn total(&self) -> usize {
        self.doc_ids.len()
    }

    /// Current position in the cursor
    pub fn position(&self) -> usize {
        self.position
    }

    /// Check if cursor is exhausted
    pub fn is_finished(&self) -> bool {
        self.position >= self.doc_ids.len()
    }

    /// Reset cursor to the beginning
    pub fn rewind(&mut self) {
        self.position = 0;
    }

    /// Skip the next N documents
    pub fn skip(&mut self, n: usize) {
        self.position = (self.position + n).min(self.doc_ids.len());
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
    /// Warning: This loads all remaining documents into memory
    pub fn collect_all(&mut self) -> Result<Vec<Value>> {
        let mut results = Vec::with_capacity(self.remaining());
        while let Some(doc) = self.next()? {
            results.push(doc);
        }
        Ok(results)
    }

    /// Take the next N documents
    pub fn take(&mut self, n: usize) -> Result<Vec<Value>> {
        let mut results = Vec::with_capacity(n);
        for _ in 0..n {
            match self.next()? {
                Some(doc) => results.push(doc),
                None => break,
            }
        }
        Ok(results)
    }
}
