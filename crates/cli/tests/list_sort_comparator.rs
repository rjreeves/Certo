use std::{fs, process::Command};

// BACKLOG item 424 — a `List.sort` comparator that read a field of a record element failed
// to compile (`member reference base type 'int64_t' is not a structure or union`): the
// lambda's params were unhinted, so they were typed as raw `int64_t`, while the runtime hands
// the comparator boxed `void*` elements and expects a raw `int64_t` back. The inline
// comparator is now lifted with boxed params, unboxed to the element type, and a raw Int
// return — the same shape `List.sortBy`'s synthesized comparator already had.

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

const PRELUDE: &str = r#"module T
import Stdlib.Core

type Item = { name: Text, qty: Int, price: Float }

fn names(xs: List<Item>): Text = {
    var out = ""
    for i in xs {
        out = out ++ i.name
    }
    out
}

fn ints(xs: List<Int>): Text = {
    var out = ""
    for x in xs {
        out = out ++ intToText(x) ++ ","
    }
    out
}

fn items(): List<Item> = [
    Item { name: "b", qty: 5, price: 2.5 }, Item { name: "a", qty: 2, price: 9.5 },
    Item { name: "c", qty: 3, price: 1.5 }, Item { name: "d", qty: 3, price: 7.5 }
]
"#;

#[test]
fn sorting_records_by_a_field_works_for_int_float_and_text_fields() {
    let src = format!("{PRELUDE}
fn main(): Unit [io] = {{
    println(names(List.sort(items(), (x, y) => x.qty - y.qty)))
    println(names(List.sort(items(), (x, y) => y.qty - x.qty)))
    println(names(List.sort(items(), (x, y) => if x.price < y.price then -1 else if x.price > y.price then 1 else 0)))
    println(names(List.sort(items(), (x, y) => if x.name < y.name then 1 else if x.name > y.name then -1 else 0)))
}}
");
    // Ties (c and d have qty 3) keep their original order: the sort is stable.
    assert_eq!(run(&src), "acdb\nbcda\ncbda\ndcba\n");
}

#[test]
fn sorting_plain_lists_and_captured_comparators_still_works() {
    let src = format!("{PRELUDE}
fn main(): Unit [io] = {{
    println(ints(List.sort([5, 3, 9, 1], (a, b) => a - b)))
    println(ints(List.sort([5, 3, 9, 1], (a, b) => b - a)))
    val words = List.sort([\"pear\", \"apple\", \"fig\"], (a, b) => if a < b then -1 else if a > b then 1 else 0)
    println(List.getOrPanic(words, 0) ++ \" \" ++ List.getOrPanic(words, 2))
    val fl = List.sort([2.5, 0.5, 1.5], (a, b) => if a < b then -1 else if a > b then 1 else 0)
    println(floatToText(List.getOrPanic(fl, 0)) ++ \" \" ++ floatToText(List.getOrPanic(fl, 2)))
    val sign = -1
    println(ints(List.sort([5, 3, 9], (a, b) => sign * (a - b))))
}}
");
    assert_eq!(run(&src), "1,3,5,9,\n9,5,3,1,\napple pear\n0.5 2.5\n9,5,3,\n");
}

#[test]
fn a_named_comparator_and_sort_by_agree_with_the_inline_one() {
    let src = format!("{PRELUDE}
fn byQty(a: Item, b: Item): Int = a.qty - b.qty

fn main(): Unit [io] = {{
    println(names(List.sort(items(), byQty)))
    println(names(List.sortBy(items(), (i) => i.qty)))
    println(names(List.sort(items(), (x, y) => x.qty - y.qty)))
}}
");
    assert_eq!(run(&src), "acdb\nacdb\nacdb\n");
}
