use std::{fs, process::Command};

// BACKLOG item 346 — Text `<`, `<=`, `>`, `>=` compiled to raw C `char*`
// pointer comparisons, so the result depended on heap addresses rather than
// the strings' contents (and `List.sort` with a `<` comparator sorted in
// allocation order). Builds and runs real programs through the compiled
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
fn text_ordering_compares_contents_not_pointers() {
    // Operands are built at runtime (`++ ""`) so they are distinct heap
    // allocations whose address order is unrelated to content order; the
    // second string is allocated first to make pointer order disagree.
    let out = build_and_run(r#"module T
fn lt(a: Text, b: Text): Bool = a < b
fn le(a: Text, b: Text): Bool = a <= b
fn gt(a: Text, b: Text): Bool = a > b
fn ge(a: Text, b: Text): Bool = a >= b
fn b2t(b: Bool): Text = if b then "T" else "F"
fn row(a: Text, b: Text): Text = b2t(lt(a, b)) ++ b2t(le(a, b)) ++ b2t(gt(a, b)) ++ b2t(ge(a, b))
fn main(): Unit [io] = {
  val hi = "banana" ++ ""
  val lo = "apple" ++ ""
  println(row(lo, hi))
  println(row(hi, lo))
  println(row(lo, "apple" ++ ""))
  println(row("app" ++ "", lo))
  println(row("Zebra" ++ "", "apple" ++ ""))
  println(row("" ++ "", lo))
  println(b2t(("apple" ++ "") < ("banana" ++ "")))
}
"#);
    assert_eq!(out, "TTFF\nFFTT\nFTFT\nTTFF\nTTFF\nTTFF\nT\n");
}

#[test]
fn sort_with_text_comparator_orders_lexicographically() {
    let out = build_and_run(r#"module T
fn main(): Unit [io] = {
  val xs = List.sort(["pear", "apple", "fig", "banana"], (p, q) => if p < q then -1 else if p > q then 1 else 0)
  println(Text.join(xs, ","))
}
"#);
    assert_eq!(out, "apple,banana,fig,pear\n");
}

#[test]
fn numeric_ordering_is_unchanged() {
    let out = build_and_run(r#"module T
fn main(): Unit [io] = {
  println(if 1 < 2 then "a" else "b")
  println(if 2.5 >= 2.5 then "a" else "b")
  println(if 3 > 4 then "a" else "b")
}
"#);
    assert_eq!(out, "a\na\nb\n");
}

#[test]
fn sort_comparators_over_non_text_elements_are_unchanged() {
    // The comparator-param hint is Text-only (the sort runtime passes raw
    // `void*` elements, so a native `double` param would read the wrong
    // register) — Float/Int comparators must keep working exactly as before.
    let out = build_and_run(r#"module T
fn main(): Unit [io] = {
  val fs = List.sort([2.5, 1.5, 3.5, 0.5], (a, b) => if a < b then -1 else if a > b then 1 else 0)
  println(Text.join(List.map(fs, (f) => floatToText(f)), ","))
  val ns = List.sort([5, 3, 9, 1], (a, b) => a - b)
  println(Text.join(List.map(ns, (n) => intToText(n)), ","))
}
"#);
    assert_eq!(out, "0.5,1.5,2.5,3.5\n1,3,5,9\n");
}
