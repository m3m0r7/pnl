//! Second native test fixture (distinct symbols from `example`), used to verify
//! `pnl compose` fuses two packages into one shared FFI scope.

use std::os::raw::c_int;

#[unsafe(no_mangle)]
pub extern "C" fn extra_triple(value: c_int) -> c_int {
    value * 3
}
