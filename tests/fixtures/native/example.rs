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

/// Cycles the `example_mode` enum (OFF=0 -> ON=1 -> AUTO=10 -> OFF). Exercises a PHP
/// enum as both a parameter and a return value (the enum is int-backed in the ABI).
#[no_mangle]
pub extern "C" fn example_next_mode(mode: c_int) -> c_int {
    match mode {
        0 => 1,
        1 => 10,
        _ => 0,
    }
}

#[repr(C)]
pub struct ExamplePoint {
    x: c_int,
    y: c_int,
}

/// Writes through a PHP-allocated `example_point`, so the PHP side can read the
/// fields back through the generated typed accessors.
#[no_mangle]
pub extern "C" fn example_point_init(point: *mut ExamplePoint, x: c_int, y: c_int) {
    unsafe {
        (*point).x = x;
        (*point).y = y;
    }
}

/// Reads a `example_point` the PHP side built and passed back in.
#[no_mangle]
pub extern "C" fn example_point_sum(point: *const ExamplePoint) -> c_int {
    unsafe { (*point).x + (*point).y }
}
