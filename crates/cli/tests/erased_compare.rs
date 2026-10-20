use std::{fs, process::Command};

// BACKLOG item 423 - ordering and equality on a bare type parameter compare the values (they used
// to compare the boxed cells' addresses), and an unannotated lambda passed to a user generic
// function takes its parameter type from the call's `List<A>` argument.
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
fn comparisons_on_a_bare_type_parameter_compare_values() {
    let out = run(r####"module P
fn maxOf<T>(a: T, b: T): T = if a > b then a else b
fn minOf<T>(a: T, b: T): T = if a < b then a else b
fn same<T>(a: T, b: T): Bool = a == b
fn differ<T>(a: T, b: T): Bool = a != b
fn viaMax<T>(a: T, b: T): T = maxOf(a, b)
fn main(): Unit [io] = {
  val x: Text = maxOf("apple", "pear")
  val y: Text = minOf("pear", "apple")
  println(x ++ " " ++ y)
  val t: Text = Text.slice("xpear", 1, 5)
  println(boolToText(same(t, "pear")) ++ " " ++ boolToText(differ(t, "pear")))
  println(boolToText(same(3, 3)) ++ " " ++ boolToText(same(3, 4)) ++ " " ++ boolToText(same(true, true)))
  println(boolToText(same(2.5, 2.5)) ++ " " ++ boolToText(differ(2.5, 3.5)))
  val f: Float = viaMax(1.5, 7.25)
  println(floatToText(f))
  val n: Int = minOf(0 - 5, 3)
  println(intToText(n))
}
"####);
    assert_eq!(out, "pear apple\ntrue false\ntrue false true\ntrue true\n7.25\n-5\n");
}

#[test]
fn an_unannotated_lambda_takes_its_type_from_a_list_argument() {
    let out = run(r####"module P
fn viaList<A, B>(xs: List<A>, f: A => B, v: A): B = f(v)
fn main(): Unit [io] = {
  val words = ["pear", "apple"]
  val t: Text = viaList(words, (s) => if s < "m" then "low" else "high", "pear")
  println(t)
  val n: Int = viaList([1, 2], (x) => x * 10, 4)
  println(intToText(n))
}
"####);
    assert_eq!(out, "high\n40\n");
}
