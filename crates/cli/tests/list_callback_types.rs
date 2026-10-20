use std::{fs, process::Command};

// BACKLOG item 438 — `List.fold`/`reduce` with a Float or record accumulator, a callback
// that read a field of a record element, and `List.partition` over records all failed in
// the C compiler (`double` passed as `void*`; a field read on an erased `int64_t`). And a
// related silent bug: a Float field read inside `filter`/`any`/`all`/`find` callbacks was
// typed `Error` and read through an `int64_t`, truncating it (`p.price > 2.5` was false
// for 2.9). Those callbacks now have typed params and go through the boxed-ABI adapters.

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
fn fold_and_reduce_work_with_float_and_record_accumulators_and_elements() {
    let out = run(r#"module T
import Stdlib.Core

type Item = { name: Text, qty: Int }
type Stats = { count: Int, total: Float }

fn main(): Unit [io] = {
    val items = [Item { name: "a", qty: 2 }, Item { name: "b", qty: 5 }, Item { name: "c", qty: 3 }]
    println(floatToText(List.fold([1.5, 2.5, 3.5], 0.0, (acc, x) => acc + x)))
    println(floatToText(List.reduce([1.5, 2.5, 3.5], 0.0, (acc, x) => acc + x)))
    println(intToText(List.fold(items, 0, (acc, i) => acc + i.qty)))
    println(intToText(List.reduce(items, 0, (m, i) => if i.qty > m then i.qty else m)))
    println(List.fold(items, Item { name: "z", qty: 0 }, (best, i) => if i.qty > best.qty then i else best).name)
    val stats = List.fold(items, Stats { count: 0, total: 0.0 }, (s, i) => Stats { count: s.count + 1, total: s.total + 1.5 })
    println(intToText(stats.count) ++ " " ++ floatToText(stats.total))
    val rate = 1.5
    println(floatToText(List.fold([2.0, 4.0], 0.0, (acc, x) => acc + x * rate)))
    println(floatToText(List.fold([], 2.5, (acc, x) => acc + 1.0)))
}
"#);
    assert_eq!(out, "7.5\n7.5\n10\n5\nb\n3 4.5\n9\n2.5\n");
}

#[test]
fn partition_works_over_records() {
    let out = run(r#"module T
import Stdlib.Core

type Item = { name: Text, qty: Int }

fn main(): Unit [io] = {
    val items = [Item { name: "a", qty: 2 }, Item { name: "b", qty: 5 }, Item { name: "c", qty: 3 }]
    val (big, small) = List.partition(items, (i) => i.qty > 2)
    println(intToText(List.len(big)) ++ "," ++ intToText(List.len(small)) ++ " " ++ List.getOrPanic(big, 0).name)
}
"#);
    assert_eq!(out, "2,1 b\n");
}

#[test]
fn a_float_field_read_in_a_callback_is_not_truncated() {
    // 2.9 > 2.5 is true; truncated to 2 it was false, so filter returned nothing.
    let out = run(r#"module T
import Stdlib.Core

type P = { name: Text, price: Float }

fn main(): Unit [io] = {
    val ps = [P { name: "a", price: 2.9 }, P { name: "b", price: 1.2 }]
    println(intToText(List.len(List.filter(ps, (p) => p.price > 2.5))))
    println(boolToText(List.any(ps, (p) => p.price > 2.8)))
    println(boolToText(List.all(ps, (p) => p.price < 3.0)))
    println((List.find(ps, (p) => p.price > 2.0) ?? P { name: "none", price: 0.0 }).name)
    val (hi, lo) = List.partition(ps, (p) => p.price > 2.5)
    println(intToText(List.len(hi)) ++ "," ++ intToText(List.len(lo)))
}
"#);
    assert_eq!(out, "1\ntrue\ntrue\na\n1,1\n");
}
