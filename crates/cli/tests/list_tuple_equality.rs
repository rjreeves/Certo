use std::{fs, process::Command};

// BACKLOG item 441 - `==` / `!=` on lists and tuples compares the elements, not the addresses.
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
fn lists_and_tuples_compare_by_value() {
    let out = run(r####"module E
type P = { x: Int, name: Text }
type Color = Red | Green
type Shape = Circle(Int) | Dot
fn main(): Unit [io] = {
  val p = P { x: 1, name: "a" }
  val q = P { x: 1, name: "a" }
  val r = P { x: 2, name: "a" }
  println(boolToText([1, 2, 3] == [1, 2, 3]))
  println(boolToText([1, 2, 3] == [1, 2]))
  println(boolToText([1, 2, 3] != [1, 2, 4]))
  println(boolToText([1.5, 2.5] == [1.5, 2.5]))
  println(boolToText(["x", "y"] == ["x", "y"]))
  println(boolToText(["x", "y"] == ["x", "z"]))
  println(boolToText([p, r] == [q, r]))
  println(boolToText([p, r] == [r, q]))
  println(boolToText([Red, Green] == [Red, Green]))
  println(boolToText([Circle(1), Circle(3)] == [Circle(1), Circle(3)]))
  println(boolToText([Circle(1), Circle(3)] == [Circle(2), Circle(3)]))
  println(boolToText([d"1.5"] == [d"1.5"]))
  println(boolToText((1, "q") == (1, "q")))
  println(boolToText((1, "q") == (2, "q")))
  println(boolToText((1, 2.5, p) == (1, 2.5, q)))
  println(boolToText((1, 2.5, p) == (1, 2.5, r)))
}
"####);
    assert_eq!(out, "true\nfalse\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\ntrue\nfalse\ntrue\nfalse\n");
}
