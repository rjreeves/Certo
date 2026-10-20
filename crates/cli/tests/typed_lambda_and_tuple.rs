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
