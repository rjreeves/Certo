// A Rust shared library called *from* Certo via `extern "C"`.
// Certo's codegen mangles `extern fn rustAdd` to the C symbol `certo_rust_add`
// (certo_ prefix + snake_case), so the export name must match exactly.
// Certo `Int` is 64-bit -> i64.

#[no_mangle]
pub extern "C" fn certo_rust_add(a: i64, b: i64) -> i64 {
    a + b
}
