//! Handle types for FFI
//!
//! These opaque handles are used to pass Rust objects across the FFI boundary.
//! C# consumers see these as IntPtr and wrap them in SafeHandle.

use ironbase_core::{CollectionCore, DatabaseCore, StorageEngine};
use std::sync::Arc;

/// Opaque database handle
///
/// Wraps an Arc<DatabaseCore<StorageEngine>> for thread-safe sharing
pub struct DatabaseHandle {
    pub(crate) inner: Arc<DatabaseCore<StorageEngine>>,
}

impl DatabaseHandle {
    pub fn new(db: DatabaseCore<StorageEngine>) -> Self {
        Self {
            inner: Arc::new(db),
        }
    }

    pub fn from_arc(db: Arc<DatabaseCore<StorageEngine>>) -> Self {
        Self { inner: db }
    }
}

/// Opaque collection handle
///
/// Holds a reference to the collection and its parent database
pub struct CollectionHandle {
    pub(crate) inner: CollectionCore<StorageEngine>,
    pub(crate) db: Arc<DatabaseCore<StorageEngine>>,
    pub(crate) name: String,
}

impl CollectionHandle {
    pub fn new(
        collection: CollectionCore<StorageEngine>,
        db: Arc<DatabaseCore<StorageEngine>>,
        name: String,
    ) -> Self {
        Self {
            inner: collection,
            db,
            name,
        }
    }
}

/// Raw pointer type for database handle (used in FFI)
pub type DbHandle = *mut DatabaseHandle;

/// Raw pointer type for collection handle (used in FFI)
pub type CollHandle = *mut CollectionHandle;

/// Validate a database handle pointer
///
/// Returns None if the pointer is null, otherwise returns a reference
#[inline]
pub(crate) fn validate_db_handle<'a>(handle: DbHandle) -> Option<&'a DatabaseHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { Some(&*handle) }
    }
}

/// Validate a database handle pointer (mutable)
#[inline]
pub(crate) fn validate_db_handle_mut<'a>(handle: DbHandle) -> Option<&'a mut DatabaseHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { Some(&mut *handle) }
    }
}

/// Validate a collection handle pointer
#[inline]
pub(crate) fn validate_coll_handle<'a>(handle: CollHandle) -> Option<&'a CollectionHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { Some(&*handle) }
    }
}

/// Validate a collection handle pointer (mutable)
#[inline]
pub(crate) fn validate_coll_handle_mut<'a>(handle: CollHandle) -> Option<&'a mut CollectionHandle> {
    if handle.is_null() {
        None
    } else {
        unsafe { Some(&mut *handle) }
    }
}
