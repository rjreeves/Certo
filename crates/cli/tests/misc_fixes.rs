use std::{fs, process::Command};

// BACKLOG items 374/375/376/412 - escapes, the → arrow, checking generated state-machine bodies,
// mutual recursion, and an @export name equal to the function's own symbol.
fn certo(src: &str, args: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).args(args).arg(&file).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout))
}

#[test]
fn unicode_escapes_keep_unknown_escapes_and_reject_nul() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module E
fn main(): Unit [io] = {
  println("smile \u{1F600} A=\u0041 caf\u00e9")
  println(intToText(Text.len("\u{1F600}")))
  println("keep \q and \\d")
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "smile \u{1F600} A=A caf\u{e9}\n4\nkeep \\q and \\d\n");

    let (ok, out) = certo(r####"module E
fn main(): Unit [io] = println("a\0b")
"####, &["check"]);
    assert!(!ok && out.contains("NUL"), "{out}");
}

#[test]
fn mutual_recursion_needs_an_annotation_in_the_cycle() {
    let (ok, out) = certo(r####"module M
fn isEven(n: Int) = if n == 0 then true else isOdd(n - 1)
fn isOdd(n: Int) = if n == 0 then false else isEven(n - 1)
fn okEven(n: Int): Bool = if n == 0 then true else okOdd(n - 1)
fn okOdd(n: Int) = if n == 0 then false else okEven(n - 1)
"####, &["check"]);
    assert!(!ok && out.matches("E0203").count() == 2 && out.contains("isEven") && !out.contains("okEven"), "{out}");
}

#[test]
fn the_spec_arrow_and_generated_bodies_in_check() {
    let (ok, out) = certo("module S\nstatemachine Sub {\n  states: Trial, Active\n  transitions:\n    Trial \u{2192} Active : activate()\n}\nfn main(): Unit [io] = println(\"ok\")\n", &["check"]);
    assert!(ok, "{out}");
    let (ok, out) = certo("module S\nstatemachine T {\n  states: A, B\n  transitions:\n    A -> B : go()\n  on_enter B: undefinedThing()\n}\nfn main(): Unit [io] = println(\"x\")\n", &["check"]);
    assert!(!ok && out.contains("undefinedThing"), "{out}");
}

#[test]
fn an_export_name_equal_to_the_functions_own_symbol_builds() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.cto");
    fs::write(&file, "module Ex\n@export(\"certo_process_order\")\npub fn processOrder(id: Text): Text = \"ok \" ++ id\n@export(\"my_add\")\npub fn addNumbers(a: Int, b: Int): Int = a + b\n").unwrap();
    let dll = dir.path().join(if cfg!(windows) { "lib.dll" } else { "lib.so" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("--emit-dll").arg("-o").arg(&dll).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    assert!(dll.exists());
}

// BACKLOG item 446 - a private constructor cannot be destructured outside its impl, and
// structural words are usable as field names.
#[test]
fn private_constructor_patterns_are_private() {
    let (ok, out) = certo(r####"module X
type Secret<T> = priv Secret(T)
impl<T> Secret {
  fn wrap(v: T): Secret<T> = Secret(v)
  fn expose(s: Secret<T>): T = match s { Secret(v) => v }
}
fn leak(s: Secret<Text>): Text = match s { Secret(v) => v }
fn leak2(s: Secret<Text>): Text = {
  val Secret(v) = s
  v
}
"####, &["check"]);
    assert!(!ok && out.matches("E0214").count() == 2, "{out}");
}

#[test]
fn structural_words_work_as_field_names() {
    let (ok, out) = certo(r####"module K
type R = { test: Int, view: Int, module: Int, type: Int, in: Int, unit: Int, when: Int }
fn main(): Unit [io] = {
  val r = R { test: 1, view: 2, module: 3, type: 4, in: 5, unit: 6, when: 7 }
  val s = r.with(test: 10)
  println(intToText(s.test + r.view + r.module + r.type + r.in + r.unit + r.when))
  match r { R { when: w, in: i, .. } => println(intToText(w + i)) }
}
"####, &["run"]);
    assert!(ok && out.replace("\r\n", "\n").trim() == "37\n12", "{out}");
}
