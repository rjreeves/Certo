// A Rust shared library called *from* Certo via `extern "C"`.
// An `extern "C"` declaration names a real C symbol, and Certo uses that name
// verbatim: `fn rustAdd` in the .cto file is the symbol `rustAdd` here, so the
// export must be spelled exactly that way (hence the camelCase).
// Certo `Int` is 64-bit -> i64.

#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn rustAdd(a: i64, b: i64) -> i64 {
    a + b
}
