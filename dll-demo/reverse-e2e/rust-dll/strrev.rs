// Rust shared library: reverses a C string. Called from Certo via `extern "C"`.
// An `extern "C"` declaration names a real C symbol and Certo uses it verbatim,
// so `fn rustReverse` in the .cto file is the symbol `rustReverse` here.
// Certo `Text` is `const char*` (UTF-8).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn rustReverse(s: *const c_char) -> *const c_char {
    if s.is_null() {
        return ptr::null();
    }
    // Borrow the incoming C string, reverse by Unicode scalar value.
    let input = unsafe { CStr::from_ptr(s) }.to_string_lossy();
    let reversed: String = input.chars().rev().collect();

    // Hand ownership to the caller. NOTE: this leaks — fine for a demo; a real
    // library would also export a `certo_rust_free(*mut c_char)` to release it.
    match CString::new(reversed) {
        Ok(c) => c.into_raw() as *const c_char,
        Err(_) => ptr::null(),
    }
}
