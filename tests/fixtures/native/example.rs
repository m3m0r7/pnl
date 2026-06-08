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
