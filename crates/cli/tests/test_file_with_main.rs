use std::{fs, process::Command};

// BACKLOG item 405 - `certo test` on a file that also has `fn main` failed to link on Windows
// (undefined CommandLineToArgvW).
#[test]
fn certo_test_runs_a_file_that_has_a_main() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.cto");
    fs::write(&file, "module T\nfn add(a: Int, b: Int): Int = a + b\nfn main(): Unit [io] = println(\"hi\")\ntest \"adds\" {\n  expect(add(1, 2)).toBe(3)\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("test").arg(&file).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("All 1 tests passed"), "{text}");
}

// BACKLOG item 406 - `certo test` ran module-level `val`s with a computed initializer as zero.
#[test]
fn module_level_vals_with_computed_initializers_are_set_in_certo_test() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.cto");
    fs::write(&file, "module T\ntype P = { a: Int, ok: Bool }\nfn double(n: Int): Int = n * 2\nval lit = 7\nval h = 3 + 4\nval g = P { a: 7, ok: true }\nval k = double(21)\n\
test \"a\" { expect(lit).toBe(7) }\ntest \"b\" { expect(h).toBe(7) }\ntest \"c\" { expect(g.a).toBe(7) }\ntest \"d\" { expect(k).toBe(42) }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("test").arg(&file).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && text.contains("All 4 tests passed"), "{text}");
}
