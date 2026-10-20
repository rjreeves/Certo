use std::{fs, process::Command};

// BACKLOG item 354 - a `var` assigned inside a lambda is shared with the enclosing function
// (it used to be copied, so the assignment was silently lost). Also covers nested lambdas, which
// must capture what only their inner lambda mentions.
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
fn a_var_assigned_in_a_lambda_is_shared_with_the_enclosing_function() {
    let out = run(r####"module P
type P = { x: Int }
fn counter(): Int = {
  var n = 0
  val bump = () => { n = n + 1 }
  bump()
  bump()
  bump()
  n
}
fn main(): Unit [io] = {
  var f = 0.5
  var s = "a"
  var p = P { x: 1 }
  var d = d"1.5"
  var flag = false
  val go = () => {
    f = f * 3.0
    s = s ++ "b"
    p = P { x: p.x + 10 }
    d = Decimal.add(d, d"2")
    flag = true
  }
  go()
  go()
  println(floatToText(f) ++ " " ++ s ++ " " ++ intToText(p.x) ++ " " ++ Decimal.toText(d) ++ " " ++ boolToText(flag))
  println(intToText(counter()))
  var total = 0
  var i = 0
  while i < 3 {
    List.forEach([1, 2, 3], (x) => { total = total + x })
    i = i + 1
  }
  println(intToText(total))
  var outer = 0
  val a = () => {
    val b = () => { outer = outer + 5 }
    b()
    b()
  }
  a()
  println(intToText(outer))
  var plain = 1
  plain = plain + 1
  println(intToText(plain))
}
"####);
    assert_eq!(out, "4.5 abb 21 5.5 true\n3\n18\n10\n2\n");
}

#[test]
fn a_nested_lambda_captures_a_variable_only_its_inner_lambda_uses() {
    let out = run(r####"module P
fn main(): Unit [io] = {
  val outer = 7
  val a = () => {
    val b = () => { println(intToText(outer)) }
    b()
  }
  a()
}
"####);
    assert_eq!(out, "7\n");
}
