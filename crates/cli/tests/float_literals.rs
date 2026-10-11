use std::{fs, process::Command};

// BACKLOG item 344 — whole-valued Float constants were printed into the
// generated C with Rust's `{:.}`, which writes `7.0` as a bare `7`. So
// `7.0 / 2.0` became C integer division (`3`), `1.0 / 3.0` became `0`, and a
// large literal like the spec's own `6.022e23` became a 24-digit integer
// clang refuses to compile. These build and run real programs through the
// compiled `certo` binary and check the observable values.
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
fn division_of_two_float_literals_is_real_division_not_integer_division() {
    let out = build_and_run(r#"module F
fn third(): Float = 1.0 / 3.0
fn half(): Float = 7.0 / 2.0
fn main(): Unit [io] = {
  println(floatToText(7.0 / 2.0))
  println(floatToText(third()))
  println(floatToText(half()))
  println(boolToText(7.0 / 2.0 == 3.5))
  println(boolToText(1.0 / 2.0 > 0.4))
  println(floatToText(10.0 - 2.5))
}
"#);
    assert_eq!(out, "3.5\n0.333333\n3.5\ntrue\ntrue\n7.5\n");
}

#[test]
fn large_and_small_float_literals_compile_and_keep_their_value() {
    // 6.022e23 is the Float literal example in the spec's own §2.4 table.
    let out = build_and_run(r#"module F
val big = 6.022e23
val huge = 1e21
val tiny = 1.5e-9
fn main(): Unit [io] = {
  println(floatToText(big))
  println(floatToText(huge))
  println(floatToText(tiny))
  val k = 6.022e23
  println(boolToText(k > 1e23))
  println(boolToText(k / 1e23 > 6.0))
}
"#);
    assert_eq!(out, "6.022e+23\n1e+21\n1.5e-09\ntrue\ntrue\n");
}

#[test]
fn whole_valued_and_negative_float_variables_still_divide_correctly() {
    let out = build_and_run(r#"module F
val whole = 7.0
val negw = -3.0
fn scale(x: Float): Float = x * 2.0
fn main(): Unit [io] = {
  println(floatToText(whole / 2.0))
  println(floatToText(negw / 2.0))
  println(floatToText(scale(1.25)))
}
"#);
    assert_eq!(out, "3.5\n-1.5\n2.5\n");
}
