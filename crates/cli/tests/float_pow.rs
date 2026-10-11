use std::{fs, process::Command};

// BACKLOG item 345 — `a ** b` on Float operands was routed to the
// integer-only `certo_pow(int64_t, int64_t)`, truncating both arguments:
// `2.0 ** 0.5` computed `certo_pow(2, 0)` = 1, `2.5 ** 2.0` gave 4,
// `10.0 ** -2.0` gave 0. Builds and runs a real program through the compiled
// `certo` binary.
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
fn float_exponentiation_is_real_floating_point_power() {
    let out = build_and_run(r#"module P
fn sq(x: Float): Float = x ** 2.0
fn root(x: Float): Float = x ** 0.5
fn main(): Unit [io] = {
  println(floatToText(2.0 ** 0.5))
  println(floatToText(2.5 ** 2.0))
  println(floatToText(9.0 ** 0.5))
  println(floatToText(10.0 ** -2.0))
  println(floatToText(sq(3.0)))
  println(floatToText(root(16.0)))
  println(floatToText(2.0 ** 3.0 ** 2.0))
  val b = 1.5
  val e = 3.0
  println(floatToText(b ** e))
}
"#);
    assert_eq!(out, "1.41421\n6.25\n3\n0.01\n9\n4\n512\n3.375\n");
}

#[test]
fn integer_exponentiation_is_unchanged() {
    let out = build_and_run(r#"module P
fn main(): Unit [io] = {
  println(intToText(2 ** 10))
  println(intToText(3 ** 4))
  println(intToText(2 ** 0))
  println(intToText(2 ** 3 ** 2))
}
"#);
    assert_eq!(out, "1024\n81\n1\n512\n");
}
