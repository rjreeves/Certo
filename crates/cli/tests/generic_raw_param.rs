use std::{fs, process::Command};

// BACKLOG item 425 — a generic function whose result paths mix a raw container element
// (`Some(v) => v`) with one of its own bare-`T` parameters (`None => d`) used to segfault: the
// two paths used different representations. The parameter is now passed as a raw element word.
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
fn generic_function_mixing_a_raw_element_and_a_parameter_works_for_every_type() {
    let out = run(r####"module P
type P = { x: Int }
fn headOr<T>(xs: List<T>, d: T): T = match xs.first() {
  Some(v) => v
  None => d
}
fn main(): Unit [io] = {
  println(intToText(headOr([5, 6], 0)))
  println(intToText(headOr([], 9)))
  println(headOr(["a"], "z"))
  println(headOr([], "dflt"))
  println(Decimal.toText(headOr([], d"1.5")))
  println(boolToText(headOr([], true)))
  println(floatToText(headOr([], 2.5)))
  println(floatToText(headOr([1.5], 2.5)))
  val p: P = headOr([], P { x: 7 })
  val q: P = headOr([P { x: 3 }], P { x: 7 })
  println(intToText(p.x) ++ intToText(q.x))
}
"####);
    assert_eq!(out, "5\n9\na\ndflt\n1.5\ntrue\n2.5\n1.5\n73\n");
}
