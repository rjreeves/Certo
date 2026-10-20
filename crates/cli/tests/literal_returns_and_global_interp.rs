use std::{fs, process::Command};

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

// BACKLOG item 373 — a function whose whole body is a Decimal or UUID literal.
#[test]
fn function_returning_a_bare_decimal_or_uuid_literal_compiles() {
    let out = run(r####"module P
fn price(): Decimal = d"1.50"
fn id(): UUID = uuid"550e8400-e29b-41d4-a716-446655440000"
fn main(): Unit [io] = {
  println(Decimal.toText(price()))
  println(UUID.toText(id()))
}
"####);
    assert_eq!(out, "1.50\n550e8400-e29b-41d4-a716-446655440000\n");
}

// BACKLOG item 372 — module-level values of every type interpolate in an f-string
// (fixed by item 434's typed globals; pinned here).
#[test]
fn module_level_values_interpolate_in_f_strings() {
    let out = run(r####"module P
val k = 5
val ratio = 2.5
val flag = true
val name = "x"
val big = k + 1
fn show(): Text = f"{k} {ratio} {flag} {name} {big}"
fn main(): Unit [io] = {
  println(f"{k}")
  println(show())
}
"####);
    assert_eq!(out, "5\n5 2.5 true x 6\n");
}
