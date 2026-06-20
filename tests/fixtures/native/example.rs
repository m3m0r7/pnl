use std::ffi::c_char;
use std::os::raw::c_int;

#[no_mangle]
pub extern "C" fn example_add(left: c_int, right: c_int) -> c_int {
    left + right
}

#[no_mangle]
pub extern "C" fn example_version() -> *const c_char {
    b"1.2.3\0".as_ptr().cast()
}

/// Invokes a C callback synchronously and returns its result plus one. Exercises a
/// PHP `callable` passed through the generated wrapper as a real C function pointer.
#[no_mangle]
pub extern "C" fn example_apply(value: c_int, callback: extern "C" fn(c_int) -> c_int) -> c_int {
    callback(value) + 1
}
