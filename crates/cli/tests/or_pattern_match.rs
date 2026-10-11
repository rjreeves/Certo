use std::{fs, process::Command};

// BACKLOG item 343 — a top-level or-pattern arm (`1 | 2 => ..`) used to fall
// into MIR's catch-all arm dispatch, which jumps to the arm body with no test
// at all, so it matched *every* scrutinee (`low(3)` returned "low"). And the
// alternatives of a binding or-pattern (`Circle(r) | Square(r) => r * r`) each
// got a separate local, so the arm body only ever read the right-hand one.
// These build and run real programs through the compiled `certo` binary.
fn build_and_run(src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

fn check_stderr(src: &str) -> (bool, String) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("check").arg(&file).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn top_level_or_pattern_only_matches_its_own_alternatives() {
    let out = build_and_run(r#"module O
type Shape = | Circle(Int) | Rect(Int, Int) | Dot
fn low(n: Int): Text = match n { 1 | 2 => "low"
  3 | 4 | 5 => "mid"
  _ => "other" }
fn k(s: Shape): Text = match s { Circle(1) | Circle(2) => "small circle"
  Circle(_) => "circle"
  Rect(1, _) | Rect(_, 1) => "thin"
  Rect(_, _) => "rect"
  Dot => "dot" }
fn b(t: Text): Int = match t { "a" | "b" => 1
  "c" => 2
  _ => 0 }
fn g(n: Int): Text = match n { 1 | 2 if n > 1 => "two"
  1 | 2 => "one"
  _ => "other" }
fn main(): Unit [io] = {
  println(low(1))
  println(low(3))
  println(low(9))
  println(k(Circle(2)))
  println(k(Circle(7)))
  println(k(Rect(9, 1)))
  println(k(Rect(4, 4)))
  println(k(Dot))
  println(intToText(b("a") + b("b") * 10 + b("c") * 100 + b("z") * 1000))
  println(g(1))
  println(g(2))
  println(g(3))
}
"#);
    assert_eq!(out, "low\nmid\nother\nsmall circle\ncircle\nthin\nrect\ndot\n211\none\ntwo\nother\n");
}

#[test]
fn or_pattern_alternatives_that_bind_the_same_name_all_feed_the_arm_body() {
    let out = build_and_run(r#"module O
type Shape = | Circle(Int) | Square(Int) | Tri(Int) | Dot
type F = | FA(Float) | FB(Float) | FC
type T = | TA(Text) | TB(Text) | TC
fn three(s: Shape): Int = match s { Circle(n) | Square(n) | Tri(n) => n + 100
  Dot => 0 }
fn guarded(s: Shape): Text = match s { Circle(n) | Square(n) if n > 5 => "big"
  Circle(_) | Square(_) => "small"
  _ => "other" }
fn orInside(o: Shape?): Int = match o { Some(Circle(n) | Square(n)) => n
  Some(_) => -1
  None => -2 }
fn tup(t: (Int, Int)): Text = match t { (0, _) | (_, 0) => "zero"
  (a, b) => "nz" ++ intToText(a + b) }
fn res(r: Result<Int, Text>): Int = match r { Ok(1) | Ok(2) => 12
  Ok(n) => n
  Err(_) => -1 }
fn fl(f: F): Float = match f { FA(x) | FB(x) => x * 2.0
  FC => 0.0 }
fn tx(t: T): Text = match t { TA(s) | TB(s) => "<" ++ s ++ ">"
  TC => "c" }
fn main(): Unit [io] = {
  println(intToText(three(Circle(1))))
  println(intToText(three(Square(2))))
  println(intToText(three(Tri(3))))
  println(guarded(Circle(9)))
  println(guarded(Square(2)))
  println(guarded(Tri(9)))
  println(intToText(orInside(Some(Circle(5)))))
  println(intToText(orInside(Some(Square(6)))))
  println(intToText(orInside(Some(Tri(7)))))
  println(tup((0, 5)))
  println(tup((2, 3)))
  println(intToText(res(Ok(2))))
  println(intToText(res(Ok(9))))
  println(floatToText(fl(FB(2.25))))
  println(tx(TA("a")))
  println(tx(TB("b")))
}
"#);
    assert_eq!(out, "101\n102\n103\nbig\nsmall\nother\n5\n6\n-1\nzero\nnz5\n12\n9\n4.5\n<a>\n<b>\n");
}

#[test]
fn or_pattern_alternatives_binding_different_names_are_rejected_with_e0223() {
    let (ok, err) = check_stderr("module O\ntype Shape = | Circle(Int) | Square(Int) | Dot\nfn f(s: Shape): Int = match s { Circle(r) | Square(q) => r\n  Dot => 0 }\nfn main(): Unit = println(\"x\")\n");
    assert!(!ok);
    assert!(err.contains("E0223"), "{err}");
}
