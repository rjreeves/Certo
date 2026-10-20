use std::{fs, process::Command};

// BACKLOG item 356 - a `val` whose name starts with a capital (spec 2.2's UPPER_SNAKE_CASE
// constants) is a binding, while a capitalised name in a `match` arm stays a constructor.
#[test]
fn upper_snake_case_values_can_be_declared_and_read() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, r####"module P
type Color = Red | Green
val MAX_RETRIES: Int = 3
val DEFAULT_NAME = "anon"
fn twice(): Int = MAX_RETRIES * 2
fn name(c: Color): Text = match c {
  Red => "r"
  Green => "g"
}
fn main(): Unit [io] = {
  val LOCAL_LIMIT = 10
  var Counter = 1
  Counter = Counter + LOCAL_LIMIT
  println(f"{twice()} {DEFAULT_NAME} {LOCAL_LIMIT} {Counter} {MAX_RETRIES}")
  println(name(Red) ++ name(Green))
}
"####).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "build failed: {}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert!(r.status.success());
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "6 anon 10 11 3\nrg\n");
}
