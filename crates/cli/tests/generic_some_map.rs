use std::{fs, process::Command};

// BACKLOG item 426 — `List.map(xs, Some)` (or `(x) => Some(x)`) inside a generic function used
// to segfault: the body cannot tell whether a raw element word needs a fresh Option cell. Such
// functions now take a hidden representation argument that every caller fills in.
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
fn generic_list_map_some_works_for_every_payload_type() {
    let out = run(r####"module P
type P = { x: Int }
fn wrapAll<T>(xs: List<T>): List<Option<T>> = List.map(xs, Some)
fn wrapLam<T>(xs: List<T>): List<Option<T>> = List.map(xs, (x) => Some(x))
fn viaWrap<T>(xs: List<T>): List<Option<T>> = wrapAll(xs)
fn main(): Unit [io] = {
  val a: List<Int?> = wrapAll([1, 2, 3])
  val b: List<Text?> = wrapAll(["a", "b"])
  val c: List<Float?> = wrapLam([1.5, 2.5])
  val d: List<Bool?> = wrapAll([true, false])
  val e: List<P?> = wrapAll([P { x: 4 }, P { x: 5 }])
  val f: List<Int?> = viaWrap([7, 8])
  val g: List<Decimal?> = wrapLam([d"1.25"])
  val h: List<Int?> = wrapAll([])
  println(intToText(List.len(h)))
  match List.getOrPanic(a, 2) { Some(v) => println(intToText(v))  None => println("none") }
  match List.getOrPanic(b, 1) { Some(v) => println(v)  None => println("none") }
  match List.getOrPanic(c, 1) { Some(v) => println(floatToText(v))  None => println("none") }
  match List.getOrPanic(d, 1) { Some(v) => println(boolToText(v))  None => println("none") }
  match List.getOrPanic(e, 1) { Some(v) => println(intToText(v.x))  None => println("none") }
  match List.getOrPanic(f, 1) { Some(v) => println(intToText(v))  None => println("none") }
  match List.getOrPanic(g, 0) { Some(v) => println(Decimal.toText(v))  None => println("none") }
}
"####);
    assert_eq!(out, "0\n3\nb\n2.5\nfalse\n5\n8\n1.25\n");
}
