use std::{fs, process::Command};

// BACKLOG item 391 — List.contains / `in` over Float, Decimal, records and sum types, and
// List.distinct over Decimal, records and sum types, use the language's `==`.
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
fn contains_and_distinct_use_value_equality_for_every_element_type() {
    let out = run(r####"module P
type P = { x: Int, y: Int }
type Shape = Circle(Int) | Square(Int)
fn main(): Unit [io] = {
  val fs = [3.5, 1.25, 2.0]
  println(boolToText(List.contains(fs, 2.0)))
  println(boolToText(List.contains(fs, 9.0)))
  println(boolToText(List.contains([d"1.5", d"2.5"], d"2.5")))
  println(boolToText(List.contains([d"1.5"], d"3")))
  println(boolToText(List.contains([P { x: 1, y: 2 }], P { x: 1, y: 2 })))
  println(boolToText(List.contains([P { x: 1, y: 2 }], P { x: 1, y: 3 })))
  println(boolToText(List.contains([Circle(1), Square(2)], Square(2))))
  println(boolToText(2.5 in [1.5, 2.5]))
  println(boolToText(List.contains([1, 2], 2)))
  println(intToText(List.len(List.distinct([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 1, y: 2 }]))))
  println(intToText(List.len(List.distinct([d"1.5", d"1.5", d"2"]))))
  println(intToText(List.len(List.distinct([Circle(1), Circle(1), Square(1)]))))
  println(intToText(List.len(List.distinct([1.5, 1.5, 2.5]))))
}
"####);
    assert_eq!(out, "true\nfalse\ntrue\nfalse\ntrue\nfalse\ntrue\ntrue\ntrue\n2\n2\n2\n2\n");
}
