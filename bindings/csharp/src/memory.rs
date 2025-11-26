//! Memory management FFI functions
//!
//! Functions for freeing memory allocated by the FFI layer

use std::ffi::CString;
use std::os::raw::c_char;

/// Free a string allocated by the FFI layer
///
/// This must be called for any string returned by `ironbase_*` functions
/// that return `*mut c_char` (except error messages from `ironbase_get_last_error()`).
///
/// # Parameters
/// - `ptr`: Pointer to the string to free
///
/// # Safety
/// - The pointer must have been allocated by an `ironbase_*` function
/// - The pointer must not be used after this call
/// - It is safe to call with a null pointer (no-op)
///
/// # Example (C#)
/// ```csharp
/// IntPtr jsonPtr = NativeMethods.ironbase_find(handle, queryPtr);
/// try {
///     string json = Marshal.PtrToStringUTF8(jsonPtr);
///     // ... use json ...
/// } finally {
///     NativeMethods.ironbase_free_string(jsonPtr);
/// }
/// ```
#[no_mangle]
pub extern "C" fn ironbase_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            // Reconstruct the CString to properly free it
            let _ = CString::from_raw(ptr);
        }
    }
}

/// Get the version of the IronBase FFI library
///
/// # Returns
/// - Version string (caller must free with `ironbase_free_string()`)
#[no_mangle]
pub extern "C" fn ironbase_version() -> *mut c_char {
    let version = env!("CARGO_PKG_VERSION");
    crate::error::string_to_c_str(version)
}
