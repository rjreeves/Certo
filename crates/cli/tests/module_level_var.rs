use std::{fs, process::Command};

// BACKLOG item 433 — a module-level `var` is accepted by the parser and type
// checker but nothing downstream emits it, so a program that used one failed in
// the C compiler with `use of undeclared identifier 'certo_counter'`. It is now
// rejected up front with E0224, by `check`, `build` and `run` alike.

const PROGRAM: &str = "module V
import Stdlib.Core

var counter = 1

fn main(): Unit [io] = {
    println(intToText(counter))
}
";

fn run(cmd: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("v.cto");
    fs::write(&file, PROGRAM).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg(cmd).arg(&file).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn check_rejects_a_module_level_var_with_e0224() {
    let (ok, text) = run("check");
    assert!(!ok, "{text}");
    assert!(text.contains("E0224"), "{text}");
    assert!(text.contains("module-level `var counter`"), "{text}");
}

#[test]
fn build_and_run_fail_with_the_certo_error_not_a_c_compiler_error() {
    for cmd in ["build", "run"] {
        let (ok, text) = run(cmd);
        assert!(!ok, "{cmd}: {text}");
        assert!(text.contains("E0224"), "{cmd}: {text}");
        assert!(!text.contains("undeclared identifier"), "{cmd} reached the C compiler: {text}");
    }
}
