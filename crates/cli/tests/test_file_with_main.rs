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
