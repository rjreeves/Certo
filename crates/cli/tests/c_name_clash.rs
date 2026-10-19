use std::{fs, process::Command};

// BACKLOG item 432 — `val fooBar` and `fn foo_bar` both mangle to the C name
// `certo_foo_bar`. That used to reach the C compiler and fail with a
// redefinition error in generated code; it is now E0225 from `check`, `build`
// and `run` alike, naming both declarations.

const CLASH: &str = "module C
import Stdlib.Core

val fooBar = 1
fn foo_bar(): Int = 2

fn main(): Unit [io] = {
    println(intToText(fooBar + foo_bar()))
}
";

fn run(cmd: &str, src: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("c.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg(cmd).arg(&file).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn check_build_and_run_report_the_clash_not_a_c_compiler_error() {
    for cmd in ["check", "build", "run"] {
        let (ok, text) = run(cmd, CLASH);
        assert!(!ok, "{cmd}: {text}");
        assert!(text.contains("E0225"), "{cmd}: {text}");
        assert!(text.contains("`fooBar` and `foo_bar` both compile to the C name `certo_foo_bar`"), "{cmd}: {text}");
        assert!(!text.contains("redefinition"), "{cmd} reached the C compiler: {text}");
    }
}

#[test]
fn renaming_one_of_them_fixes_it() {
    let (ok, text) = run("run", &CLASH.replace("foo_bar", "fooBaz"));
    assert!(ok, "{text}");
    assert!(text.contains('3'), "{text}");
}
