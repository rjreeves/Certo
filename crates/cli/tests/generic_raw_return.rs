use std::{fs, process::Command};

// BACKLOG items 350 / 340 — a user-defined generic function whose erased
// bare-`T` return is a raw container-element word (a `Some(v)` extracted
// from `List.first`/`List.last`, or `List.getOrPanic`) was unboxed at the
// call site as if it were a `BoxSome` heap cell, so any scalar / Text /
// Float instantiation dereferenced a non-pointer and segfaulted — including
// spec §5.6's own `head<T>` example. Builds and runs a real program through
// the compiled `certo` binary.
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
fn generic_functions_returning_a_raw_container_element_work_for_every_t() {
    let out = build_and_run(r####"module H
type Item = { name: Text, qty: Int }
type Box<T> = | Wrap(T)

fn head<T>(list: List<T>): T = match list.first() {
  Some(v) => v
  None => panic("empty")
}
fn firstOf<T>(xs: List<T>): T = List.getOrPanic(xs, 0)
fn viaHead<T>(xs: List<T>): T = head(xs)
fn lastOf<T>(xs: List<T>): T = match List.last(xs) {
  None => panic("empty")
  Some(v) => v
}
fn ident<T>(x: T): T = x
fn unwrapBox<T>(b: Box<T>): T = match b { Wrap(v) => v }
fn pick<T>(c: Bool, a: T, b: T): T = if c then a else b

fn main(): Unit [io] = {
  val items = [Item { name: "a", qty: 1 }, Item { name: "b", qty: 2 }]
  println(intToText(head([10, 20])))
  println(head(["x", "y"]))
  println(floatToText(head([1.5, 2.5])))
  val i_head: Item = head(items)
  println(i_head.name)
  println(intToText(firstOf([7, 8])))
  val i_firstOf: Item = firstOf(items)
  println(i_firstOf.name)
  println(firstOf(["p", "q"]))
  println(intToText(viaHead([3, 4])))
  val i_viaHead: Item = viaHead(items)
  println(i_viaHead.name)
  println(intToText(lastOf([3, 4])))
  val i_lastOf: Item = lastOf(items)
  println(i_lastOf.name)
  println(if head([true, false]) then "T" else "F")
  println(intToText(ident(5)))
  println(ident("id"))
  println(intToText(unwrapBox(Wrap(9))))
  println(intToText(pick(true, 1, 2)))
}
"####);
    assert_eq!(out, "10\nx\n1.5\na\n7\na\np\n3\na\n4\nb\nT\n5\nid\n9\n1\n");
}
