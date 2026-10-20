use std::{fs, process::Command};

// BACKLOG item 439 - a generic function can pass its own `A => Bool` parameter to a stdlib
// higher-order function, and a closure's parameter/result use a cell only where the declared
// type is a type variable.
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
fn a_generic_function_can_forward_its_closure_parameter_to_list_functions() {
    let out = run(r####"module P
type P = { n: Int }
fn pick<A>(xs: List<A>, keep: A => Bool): List<A> = List.filter(xs, keep)
fn anyOf<A>(xs: List<A>, f: A => Bool): Bool = List.any(xs, f)
fn allOf<A>(xs: List<A>, f: A => Bool): Bool = List.all(xs, f)
fn isBig(p: P): Bool = p.n > 2
fn main(): Unit [io] = {
  val ps = [P { n: 1 }, P { n: 5 }, P { n: 9 }]
  val words = ["pear", "apple", "fig"]
  val big: List<P> = pick(ps, (p) => p.n > 2)
  val named: List<P> = pick(ps, isBig)
  val ws: List<Text> = pick(words, (s) => s > "b")
  val ns: List<Int> = pick([1, 2, 3, 4], (x) => x > 2)
  println(intToText(List.len(big)) ++ " " ++ intToText(List.len(named)) ++ " " ++ intToText(List.len(ws)) ++ " " ++ intToText(List.len(ns)))
  println(boolToText(anyOf(words, (s) => s < "b")) ++ " " ++ boolToText(allOf(words, (s) => s < "b")) ++ " " ++ boolToText(allOf([2, 4], (x) => x > 1)))
}
"####);
    assert_eq!(out, "2 2 2 2\ntrue false true\n");
}

#[test]
fn closures_use_a_cell_only_where_the_declared_type_is_a_type_variable() {
    let out = run(r####"module P
type P = { n: Int }
fn check1<A>(f: A => Bool, v: A): Bool = f(v)
fn size<A>(f: A => Int, v: A): Int = f(v)
fn mixed<B>(f: Int => B, v: Int): B = f(v)
fn isBig(p: P): Bool = p.n > 2
fn dbl(n: Int): Int = n * 2
fn small(n: Int): Bool = n < 3
fn main(): Unit [io] = {
  println(boolToText(check1((x: Int) => x > 3, 5)) ++ " " ++ boolToText(check1((x: Int) => x > 3, 1)))
  println(boolToText(check1((s: Text) => s < "m", "pear")) ++ " " ++ boolToText(check1((s: Text) => s < "m", "apple")))
  println(intToText(size((s: Text) => Text.len(s), "hello")))
  val t: Text = mixed((n) => intToText(n * 2), 21)
  println(t)
  println(boolToText(check1(isBig, P { n: 9 })) ++ " " ++ boolToText(check1(isBig, P { n: 1 })))
  println(boolToText(check1(small, 2)) ++ " " ++ boolToText(check1(small, 7)))
  val d: Int = mixed(dbl, 4)
  println(intToText(d))
}
"####);
    assert_eq!(out, "true false\nfalse true\n5\n42\ntrue false\ntrue false\n8\n");
}
