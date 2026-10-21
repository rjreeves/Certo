use std::{fs, process::Command};

// BACKLOG item 349 - match exhaustiveness looks at nested patterns, so a gap inside `Some(..)` or
// `Ok(..)` is E0213 (it used to pass and segfault), and the shapes the old top-level check could not
// prove complete (records, tuples, lists) are accepted.
fn check(src: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("check").arg(&file).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout))
}

#[test]
fn a_nested_gap_is_reported_with_the_missing_value() {
    let (ok, out) = check("module A\nfn f(r: Result<Int?, Text>): Int = match r {\n  Ok(Some(x)) => x\n  Err(e) => 0\n}\n");
    assert!(!ok && out.contains("E0213") && out.contains("Ok(None)"), "{out}");

    let (ok, out) = check("module A\ntype Shape = Circle(Int) | Sq(Int)\nfn g(o: Shape?): Int = match o {\n  Some(Circle(r)) => r\n  None => 0\n}\n");
    assert!(!ok && out.contains("E0213") && out.contains("Some(Sq(_))"), "{out}");

    let (ok, out) = check("module A\nfn h(o: Int?): Int = match o {\n  Some(0) => 7\n  None => 0\n}\n");
    assert!(!ok && out.contains("E0213") && out.contains("Some(_)"), "{out}");
}

#[test]
fn complete_nested_matches_records_tuples_and_lists_are_accepted() {
    let (ok, out) = check(r#"module B
type Status = Active | Inactive | Banned
type U = { name: Text, status: Status }
fn f(u: U): Int = match u {
  { name, status: Active } => 1
  { name, status: Inactive } => 2
  { name, status: Banned } => 3
}
fn g(p: (Bool, Bool)): Int = match p {
  (true, true) => 1
  (true, false) => 2
  (false, _) => 3
}
fn l(xs: List<Int>): Int = match xs {
  [] => 0
  [x, ...rest] => x
}
fn n(r: Result<Int?, Text>): Int = match r {
  Ok(Some(x)) => x
  Ok(None) => 0
  Err(e) => 1
}
fn main(): Unit [io] = println("ok")
"#);
    assert!(ok, "{out}");
}
