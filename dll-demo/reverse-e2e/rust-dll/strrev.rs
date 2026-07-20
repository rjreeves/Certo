// Rust shared library: reverses a C string. Called from Certo via `extern "C"`.
// Certo mangles `extern fn rustReverse` -> the C symbol `certo_rust_reverse`,
// so the export name must match exactly. Certo `Text` is `const char*` (UTF-8).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

#[no_mangle]
pub extern "C" fn certo_rust_reverse(s: *const c_char) -> *const c_char {
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
