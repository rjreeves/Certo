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
    fs::write(&file, "module U\nimport Stdlib.Core\nfn main(): Unit [io] = println(Json.stringify(Json.object()))\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo-llvm"))
        .arg(&file).arg("-o").arg(dir.path().join("u.ll")).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("call to `Json.stringify` is not supported"), "{err}");
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

#[test]
fn enums_and_match_build_run_and_match_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    // A payload-free enum, a payload enum with named fields of Int, Text and
    // Float, payload-free values used where a payload type is expected, and an
    // enum stored in a record.
    let src = r#"module E
import Stdlib.Core

type Color = Red | Green | Blue
type Shape = Circle(r: Int) | Rect(w: Int, h: Int) | Label(text: Text, scale: Float) | Dot
type Tagged = { color: Color, shape: Shape }

fn name(c: Color): Text = match c {
    Red => "red"
    Green => "green"
    Blue => "blue"
}

fn area(s: Shape): Int = match s {
    Circle(r) => r * r * 3
    Rect(w, h) => w * h
    Label(t, k) => Text.len(t)
    Dot => 0
}

fn describe(s: Shape): Text = match s {
    Circle(r) => "circle " ++ intToText(r)
    Rect(w, h) => "rect " ++ intToText(w) ++ "x" ++ intToText(h)
    Label(t, k) => t ++ "@" ++ floatToText(k)
    Dot => "dot"
}

fn next(c: Color): Color = match c {
    Red => Green
    Green => Blue
    Blue => Red
}

fn main(): Unit [io] = {
    println(name(Green))
    println(name(next(Blue)))
    println(intToText(area(Rect(3, 4))))
    println(intToText(area(Circle(5))))
    println(intToText(area(Dot)))
    println(describe(Label("hi", 1.5)))
    println(describe(Rect(2, 7)))
    println(describe(Dot))
    val t = Tagged { color: Blue, shape: Circle(2) }
    println(name(t.color) ++ " " ++ describe(t.shape))
}
"#;
    let expected = "green\nred\n12\n75\n0\nhi@1.5\nrect 2x7\ndot\nblue circle 2\n";
    let llvm = build_and_run(dir.path(), src, "enum_llvm");
    assert_eq!(llvm, expected);

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if certo.exists() {
        let c_file = dir.path().join("enum_c.cto");
        fs::write(&c_file, src).unwrap();
        let c_exe = dir.path().join(if cfg!(windows) { "enum_c.exe" } else { "enum_c" });
        let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
        assert_eq!(llvm, c_out, "the LLVM and C backends disagree");
    }
}

#[test]
fn option_result_and_coalesce_build_run_and_match_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    // Options and Results over Int, Text, Float, Bool and a record, `match` on
    // both, and `??` — payloads of every width go through the same 8-byte slot.
    let src = r#"module O
import Stdlib.Core

type Item = { name: Text, qty: Int }

fn find(n: Int): Int? = if n > 0 then Some(n * 10) else None
fn pick(o: Int?): Int = match o {
    Some(v) => v
    None => 0
}
fn dflt(o: Int?): Int = o ?? 7
fn label(o: Text?): Text = o ?? "none"
fn scale(o: Float?): Float = o ?? 1.5
fn flag(o: Bool?): Bool = o ?? false

fn half(n: Int): Result<Int, Text> = if n % 2 == 0 then Ok(n / 2) else Err("odd")
fn show(r: Result<Int, Text>): Text = match r {
    Ok(v) => "ok " ++ intToText(v)
    Err(e) => "err " ++ e
}
fn find_item(n: Int): Item? = if n > 0 then Some(Item { name: "widget", qty: n }) else None
fn qty_of(o: Item?): Int = match o {
    Some(i) => i.qty
    None => -1
}

fn main(): Unit [io] = {
    println(intToText(pick(find(4))))
    println(intToText(pick(find(0))))
    println(intToText(dflt(find(2))))
    println(intToText(dflt(None)))
    println(label(Some("x")))
    println(label(None))
    println(floatToText(scale(Some(2.25))))
    println(floatToText(scale(None)))
    println(boolToText(flag(Some(true))))
    println(boolToText(flag(None)))
    println(show(half(10)))
    println(show(half(7)))
    println(intToText(qty_of(find_item(3))))
    println(intToText(qty_of(find_item(0))))
}
"#;
    let expected = "40\n0\n20\n7\nx\nnone\n2.25\n1.5\ntrue\nfalse\nok 5\nerr odd\n3\n-1\n";
    let llvm = build_and_run(dir.path(), src, "opt_llvm");
    assert_eq!(llvm, expected);

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if certo.exists() {
        let c_file = dir.path().join("opt_c.cto");
        fs::write(&c_file, src).unwrap();
        let c_exe = dir.path().join(if cfg!(windows) { "opt_c.exe" } else { "opt_c" });
        let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
        assert_eq!(llvm, c_out, "the LLVM and C backends disagree");
    }
}

#[test]
fn lists_and_tuples_build_run_and_match_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    // List literals of Int, Float, Text and records; `for` over a list; len,
    // getOrPanic, get, first, last, push (which must not change the original)
    // and tuple destructuring. The last lines use integers past 32 bits.
    let src = r#"module L
import Stdlib.Core

type Item = { name: Text, qty: Int }

fn total(xs: List<Int>): Int = {
    var t = 0
    for x in xs {
        t = t + x
    }
    t
}
fn names(xs: List<Item>): Text = {
    var out = ""
    for i in xs {
        out = out ++ i.name ++ ","
    }
    out
}
fn fsum(xs: List<Float>): Float = {
    var t = 0.0
    for x in xs {
        t = t + x
    }
    t
}
fn firstOr(xs: List<Int>): Int = List.first(xs) ?? -1
fn lastOr(xs: List<Int>): Int = List.last(xs) ?? -1
fn at(xs: List<Text>, i: Int): Text = List.get(xs, i) ?? "?"
fn pair(): (Int, Text) = (7, "seven")
fn grow(xs: List<Int>): List<Int> = List.push(List.push(xs, 100), 200)

fn main(): Unit [io] = {
    val nums = [1, 2, 3, 4]
    println(intToText(total(nums)))
    println(intToText(List.len(nums)))
    println(intToText(List.getOrPanic(nums, 2)))
    println(names([Item { name: "a", qty: 1 }, Item { name: "b", qty: 2 }]))
    println(floatToText(fsum([1.5, 2.25])))
    println(intToText(firstOr(nums)))
    println(intToText(lastOr(nums)))
    println(intToText(firstOr(List.empty())))
    val words = ["x", "y"]
    println(at(words, 1))
    println(at(words, 9))
    val g = grow(nums)
    println(intToText(List.len(g)) ++ " " ++ intToText(List.getOrPanic(g, 5)))
    println(intToText(List.len(nums)))
    val (n, s) = pair()
    println(intToText(n) ++ s)
    val big = List.push([9000000000], 5000000000)
    println(intToText(List.getOrPanic(big, 0) + List.getOrPanic(big, 1)))
}
"#;
    let expected = "10\n4\n3\na,b,\n3.75\n1\n4\n-1\ny\n?\n6 200\n4\n7seven\n14000000000\n";
    let llvm = build_and_run(dir.path(), src, "list_llvm");
    assert_eq!(llvm, expected);

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if certo.exists() {
        let c_file = dir.path().join("list_c.cto");
        fs::write(&c_file, src).unwrap();
        let c_exe = dir.path().join(if cfg!(windows) { "list_c.exe" } else { "list_c" });
        let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
        assert_eq!(llvm, c_out, "the LLVM and C backends disagree");
    }
}

#[test]
fn an_out_of_range_list_index_panics_like_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("p.cto");
    fs::write(&file, "module P\nimport Stdlib.Core\nfn main(): Unit [io] = {\n    println(\"before\")\n    println(intToText(List.getOrPanic([1, 2], 5)))\n    println(\"after\")\n}\n").unwrap();
    let exe = dir.path().join(if cfg!(windows) { "p.exe" } else { "p" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo-llvm")).arg(&file).arg("--exe").arg(&exe).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).output().unwrap();
    assert!(!run.status.success(), "an out-of-range index must not succeed");
    let out = String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n");
    let err = String::from_utf8_lossy(&run.stderr);
    assert_eq!(out, "before\n", "output written before the panic is flushed, nothing after it");
    assert!(err.contains("certo panic: list index out of bounds"), "{err}");
}

#[test]
fn arguments_environment_parse_int_and_quit_match_the_c_backend() {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let src = r#"module A
import Stdlib.Core

fn describe(n: Int?): Text = match n {
    Some(v) => "int " ++ intToText(v)
    None => "not an int"
}

fn main(): Unit [io] = {
    val first = arg(1) ?? "(none)"
    println("first: " ++ first)
    println("second: " ++ (arg(2) ?? "(none)"))
    println(describe(parseInt("42")))
    println(describe(parseInt("-7")))
    println(describe(parseInt("  12")))
    println(describe(parseInt("12abc")))
    println(describe(parseInt("")))
    println(describe(parseInt("9223372036854775807")))
    println(describe(parseInt(first)))
    println(getEnv("CERTO_TEST_VAR") ?? "unset")
    println(getEnv("CERTO_SURELY_UNSET_VAR") ?? "unset")
    if first == "quit" then Process.quit(3) else println("not quitting")
    println("done")
}
"#;
    let file = dir.path().join("a_llvm.cto");
    fs::write(&file, src).unwrap();
    let llvm_exe = dir.path().join(if cfg!(windows) { "a_llvm.exe" } else { "a_llvm" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo-llvm")).arg(&file).arg("--exe").arg(&llvm_exe).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    let c_exe = dir.path().join(if cfg!(windows) { "a_c.exe" } else { "a_c" });
    let have_c = certo.exists() && {
        let c_file = dir.path().join("a_c.cto");
        fs::write(&c_file, src).unwrap();
        Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap().status.success()
    };

    let run = |exe: &Path, args: &[&str]| {
        let out = Command::new(exe).args(args).env("CERTO_TEST_VAR", "from_env").output().unwrap();
        (String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"), out.status.code())
    };

    let (out, code) = run(&llvm_exe, &["hello", "world"]);
    assert_eq!(out, "first: hello\nsecond: world\nint 42\nint -7\nint 12\nnot an int\nnot an int\nint 9223372036854775807\nnot an int\nfrom_env\nunset\nnot quitting\ndone\n");
    assert_eq!(code, Some(0));

    let (out, _) = run(&llvm_exe, &["17"]);
    assert!(out.starts_with("first: 17\nsecond: (none)\n"), "{out}");
    assert!(out.contains("int 17\nfrom_env"), "parseInt of an argument\n{out}");

    // Process.quit stops the program with its exit code, before "done".
    let (out, code) = run(&llvm_exe, &["quit"]);
    assert_eq!(code, Some(3));
    assert!(!out.contains("done") && !out.contains("not quitting"), "{out}");

    if have_c {
        for args in [&["hello", "world"][..], &["17"], &["quit"], &[]] {
            assert_eq!(run(&llvm_exe, args), run(&c_exe, args), "the backends disagree for {args:?}");
        }
    }
}

#[test]
fn float_text_matches_the_c_backend_for_awkward_values() {
    // BACKLOG item 400: the shim's Float formatting must follow the same
    // shortest-round-trip rule as the C runtime, including where it switches
    // from plain decimal to exponents.
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let src = r#"module Fl
import Stdlib.Core

fn show(xs: List<Float>): Unit [io] = {
    for x in xs {
        println(floatToText(x))
    }
}

fn main(): Unit [io] = {
    val big = 1000000.0 * 1000000.0 * 1000000.0 * 1000000.0
    val tiny = 1.0 / 10000000.0
    show([
        3.14159265358979, 1234567.891, 123456789.125, 100000000.0, 0.1, 0.1 + 0.2,
        2.5, 3.0, -1.5, 1.0 / 3.0, 2.0 / 3.0, 0.00001234, 0.000001234,
        78.53981633974483, 1000000.0, 12345678901234567.0, 123456789012345678.0,
        big, tiny, 1.5 * tiny, 100.0, 99999.99999, 0.5, 299792458.0, -273.15, 1.1 * 1.1
    ])
}
"#;
    let llvm = build_and_run(dir.path(), src, "fl_llvm");
    assert!(llvm.starts_with("3.14159265358979\n1234567.891\n123456789.125\n100000000\n0.1\n0.30000000000000004\n"), "{llvm}");
    assert!(llvm.contains("1.2345678901234568e+17\n1e+24\n1e-07\n1.5e-07\n"), "exponent form beyond the plain-decimal range\n{llvm}");

    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if certo.exists() {
        let c_file = dir.path().join("fl_c.cto");
        fs::write(&c_file, src).unwrap();
        let c_exe = dir.path().join(if cfg!(windows) { "fl_c.exe" } else { "fl_c" });
        let build = Command::new(&certo).arg("build").arg(&c_file).arg("-o").arg(&c_exe).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let c_out = String::from_utf8_lossy(&Command::new(&c_exe).output().unwrap().stdout).replace("\r\n", "\n");
        assert_eq!(llvm, c_out, "the LLVM shim and the C runtime format Floats differently");
    }
}
