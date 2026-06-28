//! C ABI surface exported by the `cdylib` build of this crate.
//!
//! The PHP runtime loads the compiled library (expanded into `@pnlx/runtime` by
//! `pnl install`) through PHP FFI and calls these functions, so workspace JSON
//! can be re-validated against the bundled OpenAPI schemas at load time without
//! any PHP OpenAPI dependency.

use std::ffi::{CStr, CString, c_char};

use crate::model::schema::{SchemaKind, validate_json_str};

/// Validate `json` against the schema named `schema` (`pnl`, `pnlx`,
/// `pnlx-lock`, `pnlx-pathmap`, `repository-index`).
///
/// Returns a null pointer when the JSON is valid. On any problem (invalid
/// arguments, unknown schema, malformed JSON, or schema violations) it returns a
/// newly-allocated, NUL-terminated C string describing the error; the caller must
/// release it with [`pnl_string_free`].
///
/// # Safety
/// `schema` and `json` must be valid NUL-terminated C strings (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pnl_validate_json(
    schema: *const c_char,
    json: *const c_char,
) -> *mut c_char {
    let outcome = std::panic::catch_unwind(|| unsafe { validate(schema, json) });
    match outcome {
        Ok(None) => std::ptr::null_mut(),
        Ok(Some(message)) => to_c_string(&message),
        Err(_) => to_c_string("pnl: schema validation panicked"),
    }
}

/// Free a string returned by [`pnl_validate_json`].
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by [`pnl_validate_json`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pnl_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

/// Returns `None` when valid, or `Some(message)` describing the failure.
unsafe fn validate(schema: *const c_char, json: *const c_char) -> Option<String> {
    let schema = match unsafe { borrow_str(schema) } {
        Ok(value) => value,
        Err(message) => return Some(message),
    };
    let Some(kind) = SchemaKind::from_name(&schema) else {
        return Some(format!("unknown schema \"{schema}\""));
    };
    let json = match unsafe { borrow_str(json) } {
        Ok(value) => value,
        Err(message) => return Some(message),
    };

    match validate_json_str(kind, &json) {
        Ok(()) => None,
        Err(error) => Some(format!("{error:#}")),
    }
}

/// Copy a C string argument into an owned `String`, or `Err(message)` if it is
/// null or not valid UTF-8.
unsafe fn borrow_str(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("pnl: null argument passed to validator".to_owned());
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(value) => Ok(value.to_owned()),
        Err(_) => Err("pnl: argument was not valid UTF-8".to_owned()),
    }
}

fn to_c_string(message: &str) -> *mut c_char {
    // Replace interior NULs so the message always becomes a valid C string.
    CString::new(message.replace('\0', " "))
        .unwrap_or_else(|_| {
            CString::new("pnl: validation error").expect("static string has no NUL")
        })
        .into_raw()
}
