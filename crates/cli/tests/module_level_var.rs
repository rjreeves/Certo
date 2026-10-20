use std::{fs, process::Command};

// BACKLOG item 434 — mutable module-level state. A `var` declared outside any function is a
// global the program may read and reassign from any function or lambda. (It used to be
// rejected with E0224, item 433, because nothing could emit one.) Reads are typed, so an
// f-string interpolating a global Int or Float works — that used to print nothing for a
// plain module-level `val` too. Not synchronised: writing one from concurrent tasks races.

fn certo(cmd: &str, src: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg(cmd).arg(&file).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text.replace("\r\n", "\n"))
}

fn run(src: &str) -> String {
    let (ok, text) = certo("run", src);
    assert!(ok, "{text}");
    text
}

#[test]
fn module_level_vars_of_every_kind_can_be_assigned_from_functions() {
    let out = run(r#"module M
import Stdlib.Core

type P = { x: Int, label: Text }

var counter = 0
var total = 0.0
var name = "start"
var flag = false
var origin = P { x: 3, label: "o" }
var items = [1, 2]
var seed = 10 + 5
val limit = 100

fn bump(): Int = {
    counter = counter + 1
    counter
}

fn addAmount(x: Float): Unit = {
    total = total + x
}

fn rename(n: Text): Unit = {
    name = name ++ "/" ++ n
}

fn main(): Unit [io] = {
    println(intToText(bump()) ++ intToText(bump()) ++ intToText(bump()))
    addAmount(1.5)
    addAmount(2.25)
    println(floatToText(total))
    rename("a")
    rename("b")
    println(name)
    flag = true
    println(boolToText(flag))
    origin = P { x: origin.x + 1, label: "moved" }
    println(origin.label ++ intToText(origin.x))
    items = List.push(items, 3)
    println(intToText(List.len(items)))
    seed = seed * 2
    println(intToText(seed))
    println(f"counter={counter} total={total} name={name} limit={limit} x={origin.x}")
}
"#);
    assert!(out.ends_with("123\n3.75\nstart/a/b\ntrue\nmoved4\n3\n30\ncounter=3 total=3.75 name=start/a/b limit=100 x=4\n"), "{out}");
}

#[test]
fn a_var_can_be_declared_below_its_users_and_used_in_lambdas() {
    // `early` is written above `var late`, and lambdas are emitted before the globals, so
    // both used to hit "undeclared identifier" in C. Initialisers may call functions and
    // depend on earlier globals.
    let out = run(r#"module E
import Stdlib.Core

fn early(): Int = {
    late = late + 1
    late
}

var late = 40

fn compute(n: Int): Int = n * 3

var seed = compute(5)
var derived = seed + 1
var acc = 0

fn shadow(): Int = {
    val acc = 99
    acc
}

fn main(): Unit [io] = {
    println(intToText(early()))
    println(intToText(seed) ++ " " ++ intToText(derived))
    val xs = [1, 2, 3]
    val ys = List.map(xs, (x) => x + seed)
    println(intToText(List.getOrPanic(ys, 2)))
    List.forEach(xs, (x) => { acc = acc + x })
    println(intToText(acc))
    println(intToText(shadow()) ++ " " ++ intToText(acc))
    val add = (n: Int) => { acc = acc + n }
    add(10)
    println(intToText(acc))
}
"#);
    assert!(out.ends_with("41\n15 16\n18\n6\n99 6\n16\n"), "{out}");
}

#[test]
fn f_strings_can_interpolate_module_level_values() {
    // A module-level `val` of Int, Float or a record field printed nothing (or failed to
    // compile) here before: its type was unknown, so it was formatted as Text.
    let out = run(r#"module F
import Stdlib.Core

type P = { x: Int }

val limit = 100
val rate = 1.5
val origin = P { x: 3 }

fn main(): Unit [io] = {
    println(f"limit={limit} rate={rate} x={origin.x}")
}
"#);
    assert!(out.ends_with("limit=100 rate=1.5 x=3\n"), "{out}");
}

#[test]
fn assigning_a_module_level_val_is_still_rejected() {
    let (ok, text) = certo("check", "module V\nval limit = 10\nfn f(): Int = {\n    limit = 5\n    limit\n}\nfn main(): Unit = ()\n");
    assert!(!ok, "{text}");
    assert!(text.contains("E0219"), "{text}");
    assert!(text.contains("`limit`"), "{text}");
}
