// Rust consumer: calls the Certo-built add.dll.
// Certo `pub fn add` -> C symbol `certo_add`; Certo `Int` -> int64_t -> Rust `i64`.
// Links the import library `add.lib` (see the rustc -L/-l flags used to build).

#[link(name = "add")]
extern "C" {
    fn certo_add(a: i64, b: i64) -> i64;
}

fn main() {
    let (a, b) = (2_i64, 3_i64);
    let result = unsafe { certo_add(a, b) };
    println!("certo_add({a}, {b}) = {result}");
}
