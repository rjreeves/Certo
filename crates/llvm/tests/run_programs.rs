//! End-to-end: build real programs with `certo-llvm --exe` (IR + the bundled
//! runtime shim, linked by clang), run them, and compare with what the C
//! backend prints for the same source. Skipped, with a note, where no clang is
//! installed.

use std::{fs, path::Path, process::Command};

fn clang_available() -> bool {
    let found = certo_llvm::rt::find_clang().is_some();
    if !found {
        eprintln!("skipping: clang not found");
    }
    found
}

/// Builds `src` with `certo-llvm --exe` and returns what the program prints.
fn build_and_run(dir: &Path, src: &str, name: &str) -> String {
    let file = dir.join(format!("{name}.cto"));
    fs::write(&file, src).unwrap();
    let exe = dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() });
    let build = Command::new(env!("CARGO_BIN_EXE_certo-llvm"))
        .arg(&file).arg("--exe").arg(&exe).output().unwrap();
    assert!(
        build.status.success(),
        "certo-llvm failed: {}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

#[test]
fn hello_world_prints() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let out = build_and_run(dir.path(), r#"module H
import Stdlib.Core

fn greet(name: Text): Text = "Hello, " ++ name

fn main(): Unit [io] = {
    println(greet("world"))
    println(intToText(6 * 7))
}
"#, "hello");
    assert_eq!(out, "Hello, world\n42\n");
}

#[test]
fn every_supported_runtime_call_works_together() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let out = build_and_run(dir.path(), r#"module F
import Stdlib.Core

fn fact(n: Int): Int = if n <= 1 then 1 else n * fact(n - 1)
fn half(x: Float): Float = x / 2.0

fn main(): Unit [io] = {
    println(intToText(fact(10)))
    println(floatToText(half(5.0)))
    println(boolToText(fact(3) == 6))
    println(boolToText(fact(3) == 7))
    println(intToText(2 ** 10))
    println(intToText(Text.len("héllo")))
    if Text.eq("a", "a") then println("eq") else println("ne")
    if Text.eq("a", "b") then println("eq") else println("ne")
    eprintln("to stderr")
    print("no ")
    println("newline")
}
"#, "all");
    assert_eq!(out, "3628800\n2.5\ntrue\nfalse\n1024\n6\neq\nne\nno newline\n");
}

#[test]
fn output_matches_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let src = r#"module M
import Stdlib.Core

fn fib(n: Int): Int = if n < 2 then n else fib(n - 1) + fib(n - 2)

fn main(): Unit [io] = {
    println(intToText(fib(20)))
    println(floatToText(1.5 * 4.0))
    println("a" ++ "b" ++ "c")
}
"#;
    let llvm = build_and_run(dir.path(), src, "m_llvm");

    let c_file = dir.path().join("m_c.cto");
    fs::write(&c_file, src).unwrap();
    let c_exe = dir.path().join(if cfg!(windows) { "m_c.exe" } else { "m_c" });
    // The C backend's `certo` binary is in the same workspace; run it through cargo's target dir.
    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if !certo.exists() {
        eprintln!("skipping the comparison: {} is not built", certo.display());
        assert_eq!(llvm, "6765\n6\nabc\n");
        return;
    }
    let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
    assert_eq!(llvm, c_out, "the LLVM and C backends disagree");
    assert_eq!(llvm, "6765\n6\nabc\n");
}

#[test]
fn an_unsupported_call_fails_the_build_with_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("u.cto");
    fs::write(&file, "module U\nimport Stdlib.Core\nfn main(): Unit [io] = println(getEnv(\"HOME\") ?? \"none\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo-llvm"))
        .arg(&file).arg("-o").arg(dir.path().join("u.ll")).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("call to `getEnv` is not supported"), "{err}");
    assert!(!dir.path().join("u.ll").exists(), "no IR should be written when emission fails");
}

#[test]
fn records_build_run_and_match_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    // Fields listed out of declared order on purpose, plus a nested record.
    let src = r#"module R
import Stdlib.Core

type Point = { x: Int, y: Int, label: Text, scale: Float, active: Bool }
type Line = { from: Point, to: Point }

fn mk(a: Int, b: Int): Point = Point { label: "pt", y: b, x: a, active: true, scale: 1.5 }
fn sumxy(p: Point): Int = p.x + p.y
fn show(p: Point): Text = p.label ++ "(" ++ intToText(p.x) ++ "," ++ intToText(p.y) ++ ")"

fn main(): Unit [io] = {
    val p = mk(3, 4)
    println(show(p))
    println(intToText(sumxy(p)))
    println(floatToText(p.scale * 2.0))
    println(boolToText(p.active))
    val l = Line { from: p, to: mk(10, 20) }
    println(show(l.to))
    println(intToText(l.from.x + l.to.y))
}
"#;
    let expected = "pt(3,4)\n7\n3\ntrue\npt(10,20)\n23\n";
    let llvm = build_and_run(dir.path(), src, "rec_llvm");
    assert_eq!(llvm, expected);

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if certo.exists() {
        let c_file = dir.path().join("rec_c.cto");
        fs::write(&c_file, src).unwrap();
        let c_exe = dir.path().join(if cfg!(windows) { "rec_c.exe" } else { "rec_c" });
        let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
        assert_eq!(llvm, c_out, "the LLVM and C backends disagree");
    }
}
