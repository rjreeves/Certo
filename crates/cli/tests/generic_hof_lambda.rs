use std::{fs, process::Command};

// BACKLOG item 348 — an inline lambda passed to a user-defined generic higher-order function
// (spec §3.2's `apply<A, B>`) is lifted with the erased calling convention the callee expects.
fn run(src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "build failed: {}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert!(r.status.success(), "run failed: {:?}", r.status);
    String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n")
}

#[test]
fn lambdas_passed_to_user_generic_higher_order_functions_work() {
    let out = run(r####"module P
type P = { x: Int }
fn apply<A, B>(f: A => B, value: A): B = f(value)
fn twice<A>(f: A => A, v: A): A = f(f(v))
fn combine<A, B, C>(f: (A, B) => C, a: A, b: B): C = f(a, b)
fn triple(x: Int): Int = x * 3
fn main(): Unit [io] = {
  val k = 10
  val a: Int = apply((x) => x * 3, 4)
  val b: Int = apply((x: Int) => x + k, 4)
  val c: Float = apply((x: Float) => x * 1.5, 2.0)
  val d: Int = twice((x: Int) => x * 2, 5)
  val e: Text = apply((p: P) => intToText(p.x), P { x: 9 })
  val f: Int = combine((x: Int, y: Int) => x - y, 10, 4)
  val g: Text = combine((n: Int, s: Text) => s ++ intToText(n), 7, "n")
  val h: Bool = apply((x: Int) => x > 3, 4)
  val n: Int = apply(triple, 4)
  println(intToText(a) ++ " " ++ intToText(b) ++ " " ++ floatToText(c) ++ " " ++ intToText(d) ++ " " ++ e ++ " " ++ intToText(f) ++ " " ++ g ++ " " ++ boolToText(h) ++ " " ++ intToText(n))
}
"####);
    assert_eq!(out, "12 14 3 20 9 6 n7 true 12\n");
}
