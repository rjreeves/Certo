use std::{fs, process::Command};

// BACKLOG items 388/389/390 - typed lambda parameters for filter/any/find/fold/reduce/partition/sort,
// and Float/Text/Bool elements of tuples read from lists and patterns.
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
fn lambdas_over_records_floats_and_decimals_have_typed_parameters() {
    let out = run(r####"module P
type P = { name: Text, w: Float, n: Int, price: Decimal }
type Q = { p: P }
type Y = { yrs: Int }
fn main(): Unit [io] = {
  val ps = [P { name: "a", w: 1.25, n: 1, price: d"3" }, P { name: "b", w: 2.75, n: 2, price: d"9" }]
  println(intToText(List.len(List.filter(ps, (p) => p.w > 2.0))))
  println(boolToText(List.any(ps, (p) => p.w > 2.0)))
  println(boolToText(List.find(ps, (p) => p.w > 2.0) != None))
  println(intToText(List.len(List.filter(ps, (p) => Decimal.gt(p.price, d"5")))))
  val p0 = P { name: "a", w: 1.25, n: 1, price: d"3" }
  val p1 = P { name: "b", w: 2.75, n: 2, price: d"9" }
  val qs = [Q { p: p0 }, Q { p: p1 }]
  println(intToText(List.len(List.filter(qs, (q) => q.p.w > 2.0))))
  val z = List.zip([1.5, 2.5], [10.5, 20.5])
  val (f, g) = List.getOrPanic(z, 1)
  println(floatToText(f) ++ " " ++ floatToText(g))
  match List.first([(1, 2.5)]) { Some((p, q)) => println(intToText(p) ++ " " ++ floatToText(q))  None => println("none") }
  val ys = [Y { yrs: 30 }, Y { yrs: 20 }]
  println(intToText(List.fold(ys, 0, (acc, p) => acc + p.yrs)))
  println(intToText(List.reduce(ys, 0, (acc, p) => acc + p.yrs)))
  val (a, b) = List.partition(ys, (p) => p.yrs > 21)
  println(intToText(List.len(a)) ++ intToText(List.len(b)))
  val sorted = List.sort(ys, (p, q) => p.yrs - q.yrs)
  val first: Y = List.getOrPanic(sorted, 0)
  println(intToText(first.yrs))
  println(Decimal.toText(List.fold([d"1.5", d"2.5"], d"0", (a, d) => Decimal.add(a, d))))
  println(floatToText(List.fold([1.5, 2.5], 0.0, (a, x) => a + x)))
  val (c, d) = List.partition([1.5, 2.5, 3.5], (x) => x > 2.0)
  println(intToText(List.len(c)) ++ intToText(List.len(d)))
  println(intToText(List.len(List.map(List.zip([1, 2], [3, 4]), (t) => 1))))
}
"####);
    assert_eq!(out, "1\ntrue\ntrue\n1\n1\n2.5 20.5\n1 2.5\n50\n50\n11\n20\n4.0\n4\n21\n2\n");
}

#[test]
fn tuple_elements_read_from_lists_and_patterns_keep_their_types() {
    let out = run(r####"module P
fn main(): Unit [io] = {
  val z = List.zip([1.5, 2.5], [10.5, 20.5])
  val s = List.map(z, (t) => 1)
  println(intToText(List.len(s)))
  val w = List.zip(["a", "b"], [true, false])
  val (n, flag) = List.getOrPanic(w, 1)
  println(n ++ " " ++ boolToText(flag))
  match (1, 2.5, "x") {
    (a, b, c) => println(intToText(a) ++ " " ++ floatToText(b) ++ " " ++ c)
  }
  match List.first([(1, (2.5, "y"))]) {
    Some((a, (b, c))) => println(floatToText(b) ++ c)
    None => println("none")
  }
}
"####);
    assert_eq!(out, "2\nb false\n1 2.5 x\n2.5y\n");
}

// BACKLOG item 437 - an annotated parameter of a lambda bound with `val` keeps its type, and a
// call through the lambda has the type of its body (it printed `2` for 2.5).
#[test]
fn an_annotated_lambda_bound_to_a_val_keeps_its_float_type() {
    let out = run(r####"module T
fn main(): Unit [io] = {
  val half = (x: Float) => x / 2.0
  println(floatToText(half(5.0)))
  val scale = (x: Float, k: Int) => x * 2.0
  println(floatToText(scale(1.25, 3)))
}
"####);
    assert_eq!(out, "2.5\n2.5\n");
}

// BACKLOG item 368 - annotated lambda values keep their parameter types (Float, Text, Decimal, a
// record), and a curried local lambda works.
#[test]
fn annotated_lambda_values_keep_their_parameter_types() {
    let out = run(r####"module A
type P = { w: Float, n: Int }
fn main(): Unit [io] = {
  val h = (a: Float, b: Float) => a / b
  println(floatToText(h(1.0, 4.0)))
  val t = (a: Text, b: Text) => a == b
  println(boolToText(t("x" ++ "y", "xy")))
  val lt = (a: Text, b: Text) => a < b
  println(boolToText(lt("apple", "fig")))
  val d = (a: Decimal) => Decimal.add(a, d"1.5")
  println(Decimal.toText(d(d"2")))
  val f = (x: P) => x.w > 2.0
  println(boolToText(f(P { w: 3.0, n: 1 })))
  val sub = (a: Int) => (b: Int) => a - b
  println(intToText(sub(1)(10)))
  val inc = sub(100)
  println(intToText(inc(1)))
  val mk = (x: P) => P { w: x.w * 2.0, n: x.n + 1 }
  println(floatToText(mk(P { w: 1.5, n: 1 }).w))
}
"####);
    assert_eq!(out, "0.25\ntrue\ntrue\n3.5\ntrue\n-9\n99\n3\n");
}
