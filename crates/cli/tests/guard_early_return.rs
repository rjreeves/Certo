use std::{fs, process::Command};

// BACKLOG item 342 — `guard cond else e` used to lower to `if !cond { e }`,
// which evaluated `e` and discarded it without ever leaving the function, so
// every `guard` was a silent no-op (`g(-1)` printed `Ok -1`). These build and
// run a real program through the compiled `certo` binary and check the
// observable behavior, not the generated code's shape.
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

#[test]
fn guard_returns_its_else_value_early_instead_of_falling_through() {
    let out = build_and_run(r#"module G
fn g(amount: Int, max: Int): Result<Int, Text> = {
  guard amount > 0 else Err("amount must be positive")
  guard amount <= max else Err("amount exceeds maximum")
  Ok(amount)
}
fn show(r: Result<Int, Text>): Unit [io] = match r {
  Ok(v) => println(f"Ok {v}")
  Err(e) => println(f"Err {e}")
}
fn main(): Unit [io] = {
  show(g(-1, 10))
  show(g(50, 10))
  show(g(5, 10))
}
"#);
    assert_eq!(out, "Err amount must be positive\nErr amount exceeds maximum\nOk 5\n");
}

#[test]
fn guard_skips_the_rest_of_the_body_and_still_runs_defers() {
    let out = build_and_run(r#"module G
fn noisy(open: Bool): Result<Int, Text> [io] = {
  defer { println("cleanup") }
  guard open else Err("closed")
  println("reached the end")
  Ok(1)
}
fn main(): Unit [io] = {
  match noisy(false) { Ok(v) => println("ok")
    Err(e) => println(e) }
  match noisy(true) { Ok(v) => println("ok")
    Err(e) => println(e) }
}
"#);
    // The early-return path must not print "reached the end", but must run the defer.
    assert_eq!(out, "cleanup\nclosed\nreached the end\ncleanup\nok\n");
}

#[test]
fn guard_works_for_non_pointer_return_types_in_a_loop_and_an_impl_method() {
    let out = build_and_run(r#"module G
import Stdlib.Collections.{ List }
type Cart = { open: Bool, qty: Int }
fn half(x: Float): Float = {
  guard x > 0.0 else -1.5
  x / 2.0
}
fn firstBig(xs: List<Int>): Int = {
  for x in xs {
    guard x < 10 else x
  }
  0 - 1
}
impl Cart {
  fn add(self, q: Int): Result<Int, Text> = {
    guard self.open else Err("closed")
    guard q > 0 else Err("badqty")
    Ok(self.qty + q)
  }
}
fn main(): Unit [io] = {
  println(floatToText(half(-3.0)))
  println(floatToText(half(5.0)))
  println(intToText(firstBig([1, 2, 30, 4])))
  println(intToText(firstBig([1, 2])))
  val c = Cart { open: true, qty: 10 }
  match c.add(0) { Ok(v) => println("ok")
    Err(e) => println(e) }
  match c.add(4) { Ok(v) => println(intToText(v))
    Err(e) => println(e) }
}
"#);
    assert_eq!(out, "-1.5\n2.5\n30\n-1\nbadqty\n14\n");
}

#[test]
fn guard_inside_a_lambda_is_rejected_with_e0222() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, "module G\nimport Stdlib.Collections.{ List }\nfn f(xs: List<Int>): List<Int> = List.map(xs, (x) => {\n  guard x > 0 else 0\n  x\n})\nfn main(): Unit = println(\"x\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("check").arg(&file).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("E0222"), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_variable_used_only_in_a_guards_else_value_is_not_reported_unused_by_lint() {
    // The guard's else value now sits inside a `Return` node; the lint
    // walkers must descend into it or they flag `msg` as unused.
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, "module G\nfn f(ok: Bool): Result<Int, Text> = {\n  val msg = \"closed\"\n  guard ok else Err(msg)\n  Ok(1)\n}\nfn main(): Unit = println(\"x\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("lint").arg(&file).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!text.contains("unused variable"), "{text}");
}
