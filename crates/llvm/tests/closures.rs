//! BACKLOG item 437 - closures, higher-order list functions and the other language features
//! the LLVM backend used to reject. Each program is built with `certo-llvm --exe` and with the
//! C backend, and the two outputs must agree.

use std::{fs, path::Path, process::Command};

fn clang_available() -> bool {
    let found = certo_llvm::rt::find_clang().is_some();
    if !found {
        eprintln!("skipping: clang not found");
    }
    found
}

fn exe_name(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

/// Output of `src` built with the LLVM backend.
fn llvm_output(dir: &Path, src: &str) -> String {
    let file = dir.join("p_llvm.cto");
    fs::write(&file, src).unwrap();
    let exe = exe_name(dir, "p_llvm");
    let build = Command::new(env!("CARGO_BIN_EXE_certo-llvm")).arg(&file).arg("--exe").arg(&exe).output().unwrap();
    assert!(build.status.success(), "certo-llvm failed: {}{}",
        String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

/// Output of the same program from the C backend, when it is built.
fn c_output(dir: &Path, src: &str) -> Option<String> {
    let certo = Path::new(env!("CARGO_BIN_EXE_certo-llvm")).with_file_name(if cfg!(windows) { "certo.exe" } else { "certo" });
    if !certo.exists() { return None; }
    let file = dir.join("p_c.cto");
    fs::write(&file, src).unwrap();
    let exe = exe_name(dir, "p_c");
    let build = Command::new(&certo).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    Some(String::from_utf8_lossy(&Command::new(&exe).output().unwrap().stdout).replace("\r\n", "\n"))
}

fn check(src: &str, expected: &str) {
    if !clang_available() { return; }
    let dir = tempfile::tempdir().unwrap();
    let llvm = llvm_output(dir.path(), src);
    assert_eq!(llvm, expected);
    if let Some(c) = c_output(dir.path(), src) {
        assert_eq!(llvm, c, "the LLVM and C backends disagree");
    }
}

#[test]
fn lambdas_capture_and_pass_through_functions() {
    check(r#"module C
import Stdlib.Core

fn apply(f: Int => Int, x: Int): Int = f(x)
fn twice(f: Int => Int, x: Int): Int = f(f(x))
fn double(n: Int): Int = n * 2
fn adder(k: Int): Int => Int = (x: Int) => x + k

fn main(): Unit [io] = {
    val k = 10
    val label = "n="
    val add = (x: Int) => x + k
    println(intToText(apply(add, 5)))
    println(intToText(twice(add, 5)))
    println(intToText(apply(double, 21)))
    println(intToText(adder(3)(4)))
    val show = (x: Int) => label ++ intToText(x)
    println(show(7))
    val half = (x: Float) => x / 2.0
    println(floatToText(half(5.0)))
}
"#, "15\n25\n42\n7\nn=7\n2.5\n");
}

#[test]
fn higher_order_list_functions_run() {
    check(r#"module L
import Stdlib.Core

type Item = { name: Text, qty: Int }

fn main(): Unit [io] = {
    val xs = [1, 2, 3, 4, 5, 6]
    val k = 3
    val doubled = List.map(xs, (n) => n * 2)
    val big = List.filter(xs, (n) => n > k)
    println(intToText(List.fold(doubled, 0, (a, b) => a + b)))
    println(intToText(List.len(big)))
    println(boolToText(List.any(xs, (n) => n == 4)))
    println(boolToText(List.all(xs, (n) => n < 6)))
    println(intToText(List.find(xs, (n) => n > 4) ?? -1))
    println(intToText(List.find(xs, (n) => n > 40) ?? -1))
    var seen = 0
    List.forEach(xs, (n) => { seen = seen + n })
    println(intToText(seen))
    val halves = List.map(xs, (n) => 1.5 * 2.0)
    println(floatToText(List.fold(halves, 0.0, (a, b) => a + b)))
    val names = List.map([Item { name: "a", qty: 1 }, Item { name: "b", qty: 2 }], (i) => i.name ++ intToText(i.qty))
    println(List.fold(names, "", (a, b) => a ++ b ++ ";"))
}
"#, "42\n3\ntrue\nfalse\n5\n-1\n21\n18\na1;b2;\n");
}

#[test]
fn generic_sum_types_and_recursive_variants_run() {
    check(r#"module G
import Stdlib.Core

type Maybe<T> = Just(T) | Nothing
type Tree = Leaf | Node(Tree, Int, Tree)

fn orZero(m: Maybe<Int>): Int = match m {
    Just(v) => v
    Nothing => 0
}
fn insert(t: Tree, v: Int): Tree = match t {
    Leaf => Node(Leaf, v, Leaf)
    Node(l, x, r) => if v < x then Node(insert(l, v), x, r) else Node(l, x, insert(r, v))
}
fn sum(t: Tree): Int = match t {
    Leaf => 0
    Node(l, x, r) => sum(l) + x + sum(r)
}
fn depth(t: Tree): Int = match t {
    Leaf => 0
    Node(l, _, r) => 1 + (if depth(l) > depth(r) then depth(l) else depth(r))
}

fn main(): Unit [io] = {
    println(intToText(orZero(Just(5))))
    println(intToText(orZero(Nothing)))
    match Just(2.5) {
        Just(x) => println(floatToText(x))
        Nothing => println("none")
    }
    match Just("hi") {
        Just(s) => println(s)
        Nothing => println("none")
    }
    val t = insert(insert(insert(insert(Leaf, 5), 3), 8), 1)
    println(intToText(sum(t)))
    println(intToText(depth(t)))
}
"#, "5\n0\n2.5\nhi\n17\n3\n");
}

#[test]
fn structural_equality_on_records_and_enums_matches_the_c_backend() {
    check(r#"module E
import Stdlib.Core

type P = { x: Int, name: Text, w: Float }
type Line = { a: P, b: P }
type Color = Red | Green | Rgb(Int, Int, Int)

fn main(): Unit [io] = {
    val a = P { x: 1, name: "a", w: 1.5 }
    val b = P { x: 1, name: "a", w: 1.5 }
    val c = P { x: 1, name: "b", w: 1.5 }
    println(boolToText(a == b))
    println(boolToText(a == c))
    println(boolToText(a != c))
    println(boolToText(Line { a: a, b: c } == Line { a: b, b: c }))
    println(boolToText(Line { a: a, b: c } == Line { a: c, b: c }))
    println(boolToText(Red == Red))
    println(boolToText(Red == Green))
    println(boolToText(Rgb(1, 2, 3) == Rgb(1, 2, 3)))
    println(boolToText(Rgb(1, 2, 3) == Rgb(1, 2, 4)))
    println(boolToText(Rgb(1, 2, 3) == Red))
}
"#, "true
false
true
true
false
true
false
true
false
false
");
}

#[test]
fn structural_equality_on_lists_and_tuples() {
    check(r#"module E
import Stdlib.Core

type P = { x: Int }

fn main(): Unit [io] = {
    val a = P { x: 1 }
    val b = P { x: 1 }
    val c = P { x: 2 }
    println(boolToText([1, 2, 3] == [1, 2, 3]))
    println(boolToText([1, 2, 3] == [1, 2]))
    println(boolToText([a, c] == [b, c]))
    println(boolToText([a, c] == [c, a]))
    println(boolToText(["x", "y"] == ["x", "y"]))
    println(boolToText((1, "q") == (1, "q")))
    println(boolToText((1, "q") == (2, "q")))
}
"#, "true
false
true
false
true
true
false
");
}

#[test]
fn a_var_assigned_in_a_lambda_is_shared_and_closures_nest() {
    check(r#"module V
import Stdlib.Core

fn counter(): Int = {
    var n = 0
    val bump = () => { n = n + 1 }
    bump()
    bump()
    bump()
    n
}

fn main(): Unit [io] = {
    println(intToText(counter()))
    var total = 0
    List.forEach([1, 2, 3], (x) => { total = total + x })
    println(intToText(total))
    var outer = 0
    val a = () => {
        val b = () => { outer = outer + 5 }
        b()
        b()
    }
    a()
    println(intToText(outer))
}
"#, "3\n6\n10\n");
}

#[test]
fn more_list_functions_run() {
    check(r#"module M
import Stdlib.Core

fn show(xs: List<Int>): Text = List.fold(xs, "", (a, b) => a ++ intToText(b) ++ ",")
fn showT(xs: List<Text>): Text = List.fold(xs, "", (a, b) => a ++ b ++ ",")

fn head(xs: List<Int>): Int = match xs {
    [] => -1
    [x, ...rest] => x + List.len(rest) * 100
    _ => -2
}

fn main(): Unit [io] = {
    val xs = [3, 1, 2, 3, 1]
    println(show(List.reverse(xs)))
    println(show(List.concat(xs, [9])))
    println(show(List.slice(xs, 1, 3)))
    println(show(List.distinct(xs)))
    println(showT(List.distinct(["a", "b", "a"])))
    println(boolToText(List.contains(xs, 2)))
    println(boolToText(List.contains(xs, 7)))
    println(boolToText(List.contains(["a", "b"], "b")))
    println(show(List.sort(xs, (a, b) => a - b)))
    println(show(List.sort(xs, (a, b) => b - a)))
    val (small, big) = List.partition(xs, (n) => n < 2)
    println(show(small) ++ "|" ++ show(big))
    println(show(List.flatMap([1, 2], (n) => [n, n * 10])))
    println(intToText(List.len(List.chunked(xs, 2))))
    println(intToText(head([5, 6, 7])))
    println(intToText(head([])))
}
"#, "1,3,2,1,3,\n3,1,2,3,1,9,\n1,2,\n3,1,2,\na,b,\ntrue\nfalse\ntrue\n1,1,2,3,3,\n3,3,2,1,1,\n1,1,|3,2,3,\n1,10,2,20,\n3\n205\n-1\n");
}
