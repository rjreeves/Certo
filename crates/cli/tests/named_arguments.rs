use std::{fs, process::Command};

// BACKLOG item 352 - named arguments bind by name for constructors too, and an unknown or repeated
// name is an error (E0227 / E0228) instead of binding positionally.
fn certo(src: &str, args: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).args(args).arg(&file).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout))
}

#[test]
fn constructor_and_function_arguments_bind_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module N
type Shape = Rect(width: Int, height: Int) | Dot
type Pt = { a: Int, b: Int }
fn paged(q: Int, page: Int = 0, size: Int = 20): Int = q * 10000 + page * 100 + size
fn main(): Unit [io] = {
  val r = Rect(height: 3, width: 10)
  match r {
    Rect(w, h) => println(intToText(w - h))
    Dot => println("dot")
  }
  println(intToText(Pt(b: 1, a: 9).a))
  println(intToText(paged(1, size: 5)))
  println(intToText(paged(2, size: 5, page: 3)))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "7\n9\n10005\n20305\n");
}

#[test]
fn unknown_and_repeated_argument_names_are_errors() {
    let head = "module N\ntype Shape = Rect(width: Int, height: Int) | Dot\nfn paged(q: Int, page: Int = 0, size: Int = 20): Int = q\nfn main(): Unit [io] = {\n";
    for (body, code) in [
        ("val r = Rect(bogus: 10, height: 3)", "E0227"),
        ("println(intToText(paged(1, bogus: 3)))", "E0227"),
        ("println(intToText(paged(1, 2, page: 3)))", "E0228"),
        ("println(intToText(paged(1, page: 2, page: 3)))", "E0228"),
    ] {
        let (ok, out) = certo(&format!("{head}  {body}\n}}\n"), &["check"]);
        assert!(!ok && out.contains(code), "{body}: {out}");
    }
}

// BACKLOG item 363 - a function declaration with no body is an error, not `return 0;`.
#[test]
fn a_function_without_a_body_is_an_error() {
    let (ok, out) = certo("module B\nfn compute(x: Int): Int\nfn main(): Unit [io] = println(intToText(compute(3)))\n", &["check"]);
    assert!(!ok && out.contains("E0229") && out.contains("`compute` has no body"), "{out}");
}

// BACKLOG item 362 - arithmetic needs numbers, record patterns need real fields, and a name can be
// declared once per namespace.
#[test]
fn bad_operands_unknown_pattern_fields_and_duplicate_declarations_are_errors() {
    for (src, code) in [
        ("module A\nfn f(): Bool = true + false\n", "E0230"),
        ("module A\nfn f(): Text = \"a\" - \"b\"\n", "E0230"),
        ("module A\nfn f(): Int = [1] + [2]\n", "E0230"),
        ("module A\ntype P = { a: Int }\nfn f(p: P): Int = { val { zz } = p\n  1 }\n", "E0231"),
        ("module A\nfn dup(): Int = 1\nfn dup(): Int = 2\n", "E0232"),
        ("module A\ntype T = { a: Int }\ntype T = { b: Int }\n", "E0232"),
    ] {
        let (ok, out) = certo(src, &["check"]);
        assert!(!ok && out.contains(code), "{src}: {out}");
    }
    // A function and a type may share a spelling, and a Duration adds.
    let (ok, out) = certo("module A\ntype Pt = { a: Int }\nfn pt(): Int = 1\nfn d(a: Duration, b: Duration): Duration = a + b\nfn main(): Unit [io] = println(\"ok\")\n", &["check"]);
    assert!(ok, "{out}");
}

// BACKLOG item 361 - an anonymous record literal `{ x: 1, y: 2 }` is the record type its field
// names pick out; none or several matches is E0233.
#[test]
fn anonymous_record_literals_resolve_by_field_names() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module R
type Pt = { x: Int, y: Int }
type Person = { name: Text, age: Int }
fn sum(p: Pt): Int = p.x + p.y
fn main(): Unit [io] = {
  val a = { x: 1, y: 2 }
  val who = { name: "Ann", age: 30 }
  println(intToText(a.x + sum({ x: 3, y: 4 })))
  println(who.name ++ intToText(who.age))
  val block = { val z = 5
    z + 1 }
  println(intToText(block))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "8\nAnn30\n6\n");

    let (ok, out) = certo("module R\ntype Pt = { x: Int, y: Int }\nfn main(): Unit [io] = { val a = { q: 1 }\n  println(\"x\") }\n", &["check"]);
    assert!(!ok && out.contains("E0233"), "{out}");
    let (ok, out) = certo("module R\ntype A = { x: Int, y: Int }\ntype B = { x: Int, z: Int }\nfn main(): Unit [io] = { val a = { x: 1 }\n  println(\"x\") }\n", &["check"]);
    assert!(!ok && out.contains("E0233") && out.contains("A, B"), "{out}");
}

// BACKLOG item 365 - the range extremes of spec §4.1's table can be written.
#[test]
fn int_and_uint_range_extreme_literals() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module I
fn main(): Unit [io] = {
  val c: UInt = 18446744073709551615
  val h: UInt = 0xFFFFFFFFFFFFFFFF
  val m: UInt = 9223372036854775808
  val mx: Int = 9223372036854775807
  val mn = -9223372036854775808
  println(f"{c} {h} {m}")
  println(intToText(mx))
  println(intToText(mn))
  println(boolToText(c == h))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"),
        "18446744073709551615 18446744073709551615 9223372036854775808\n9223372036854775807\n-9223372036854775808\ntrue\n");
    // An Int cannot hold a UInt-sized literal, and nothing holds 2^64.
    let (ok, out) = certo("module I\nfn main(): Unit [io] = { val x: Int = 18446744073709551615\n println(\"x\") }\n", &["check"]);
    assert!(!ok && out.contains("UInt"), "{out}");
    let (ok, out) = certo("module I\nfn main(): Unit [io] = { val x: UInt = 18446744073709551616\n println(\"x\") }\n", &["check"]);
    assert!(!ok && out.contains("too large"), "{out}");
}

// BACKLOG item 364 - `impl Trait<Args> for Type<Args>` parses, runs and survives `certo fmt`.
#[test]
fn generic_trait_impls_parse_run_and_format() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module T
trait Functor<F> {
  fn fmap(x: F): F
}
trait Container<A> {
  fn first(c: Self): A
}
type Box = { v: Int }
type Pair = { a: Int, b: Int }
impl Functor<Box> for Box {
  fn fmap(x: Box): Box = Box { v: x.v + 1 }
}
impl Container<List<Int>> for Pair {
  fn first(c: Pair): List<Int> = [c.a]
}
fn main(): Unit [io] = {
  println(intToText(Box.fmap(Box { v: 1 }).v))
  println(intToText(List.len(Pair.first(Pair { a: 7, b: 8 }))))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "2\n1\n");
    let f = Command::new(env!("CARGO_BIN_EXE_certo")).arg("fmt").arg(&file).output().unwrap();
    assert!(f.status.success());
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("impl Functor<Box> for Box {") && text.contains("impl Container<List<Int>> for Pair {"), "{text}");
}

// BACKLOG item 371 - a nullary variant of a sum type that also has payload variants can sit in a
// list, a tuple, an Option/Result payload or a `List.push` argument.
#[test]
fn nullary_variants_of_payload_sum_types_in_lists_and_tuples() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module S
type Shape = Circle(Int) | Dot | Sq(Int, Int)
type OrderError = NoLines | Over(Int, Int)
type Order = { lines: Int }
validator Submit for Order errors OrderError {
  rule has_lines {
    require order.lines > 0
    else OrderError.NoLines
  }
}
fn area(s: Shape): Int = match s {
  Circle(r) => r
  Dot => 0
  Sq(a, b) => a * b
}
fn pick(n: Int): Shape? = if n > 0 then Some(Dot) else None
fn res(n: Int): Result<Shape, Text> = if n > 0 then Ok(Dot) else Err("neg")
fn main(): Unit [io] = {
  var xs: List<Shape> = []
  xs = List.push(xs, Dot)
  xs = List.push(xs, Sq(2, 3))
  println(intToText(List.fold(xs, 0, (a, s) => a + area(s))))
  val (s, n) = (Dot, 5)
  println(intToText(area(s) + n))
  println(intToText(List.len(List.filter([Dot, Circle(2), Dot], x => area(x) == 0))))
  match pick(1) {
    Some(v) => println(intToText(area(v)))
    None => println("none")
  }
  match res(1) {
    Ok(v) => println(intToText(area(v)))
    Err(e) => println(e)
  }
  match Submit.validate(Order { lines: 0 }) {
    Ok(o) => println("ok")
    Err(e) => println("invalid")
  }
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "6\n5\n2\n0\n0\ninvalid\n");
}
