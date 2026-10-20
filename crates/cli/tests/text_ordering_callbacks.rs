use std::{fs, process::Command};

// BACKLOG item 423 — `<`/`>` on two lambda parameters compared pointers instead of
// text when the parameters were unhinted: `List.reduce(words, "zzz", (acc, w) =>
// if w < acc then w else acc)` returned "zzz". A lambda param with no type stays
// `Ty::Error` through HIR, so the comparison could not be recognised as Text. The
// callbacks of filter/find/any/all/partition/fold/reduce now hint their params
// where they are Text (and only there: the runtime calls these callbacks with raw
// `void*` elements, so a native double or struct param would use the wrong ABI).

fn run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.cto");
    fs::write(&file, src).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "t.exe" } else { "t" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn text_comparison_inside_fold_and_reduce_callbacks_orders_lexicographically() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    val words = ["pear", "apple", "fig", "banana"]
    println(List.reduce(words, "zzz", (acc, w) => if w < acc then w else acc))
    println(List.fold(words, "zzz", (acc, w) => if w < acc then w else acc))
    println(List.reduce(words, "", (acc, w) => if w > acc then w else acc))
    val a = "pear"
    println(List.fold(words, a, (lo, w) => if w < lo then w else lo))
}
"#);
    assert_eq!(out, "apple\napple\npear\napple\n");
}

#[test]
fn text_comparison_inside_predicate_callbacks_orders_lexicographically() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    val words = ["pear", "apple", "fig", "banana"]
    val (early, late) = List.partition(words, (w) => w < "c")
    println(intToText(List.len(early)) ++ " " ++ intToText(List.len(late)))
    println(boolToText(List.any(words, (a) => a < "b")))
    println(boolToText(List.all(words, (a) => a < "z")))
    println(List.getOrPanic(List.filter(words, (w) => w > "f"), 0))
    println(List.find(words, (w) => w > "o") ?? "?")
}
"#);
    assert_eq!(out, "2 2\ntrue\ntrue\npear\npear\n");
}

#[test]
fn callbacks_over_other_element_types_are_unchanged() {
    // Int and Float callbacks give the same answers as before. (Records, and a Float
    // accumulator in `fold`, are covered by `list_callback_types.rs`: they used to fail
    // to compile, see BACKLOG item 438.)
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    println(intToText(List.reduce([3, 1, 2], 100, (acc, w) => if w < acc then w else acc)))
    println(boolToText(List.any([1.5, 2.5], (x) => x > 2.0)))
    println(intToText(List.len(List.filter([5, 1, 7], (n) => n > 2))))
    println(intToText(List.fold([1, 2, 3], 10, (acc, n) => acc + n)))
}
"#);
    assert_eq!(out, "1
true
2
16
");
}
