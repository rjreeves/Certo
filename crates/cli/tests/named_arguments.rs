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

// BACKLOG item 366 - `await` of a plain value is an error; an async call still runs synchronously.
#[test]
fn await_needs_a_task_or_a_call() {
    let (ok, out) = certo("module A\nasync fn fetch(id: Int): Int = id + 1\nfn main(): Unit [io, async] = {\n  val x = await 5\n  println(intToText(x))\n}\n", &["check"]);
    assert!(!ok && out.contains("E0234"), "{out}");
    let (ok, out) = certo("module A\nasync fn fetch(id: Int): Int = id + 1\nfn main(): Unit [io, async] = {\n  val v = fetch(3)\n  println(intToText(v))\n  val w = await fetch(4)\n  println(intToText(w))\n  val t = spawn fetch(5)\n  println(intToText(await t))\n}\n", &["check"]);
    assert!(ok, "{out}");
}

// BACKLOG item 370 - `.with(field: value)` checks the value against the field's type.
#[test]
fn with_checks_the_types_of_the_overriding_values() {
    let (ok, out) = certo("module W\ntype P = { name: Text, age: Int }\nfn main(): Unit [io] = {\n  val p = P { name: \"a\", age: 1 }\n  val q = p.with(age: \"old\", name: 7)\n  println(q.name)\n}\n", &["check"]);
    assert!(!ok && out.contains("E0200"), "{out}");
    let (ok, out) = certo("module W\ntype P = { name: Text, age: Int }\ntype Box<T> = { v: T, n: Int }\nfn main(): Unit [io] = {\n  val p = P { name: \"a\", age: 1 }\n  val q = p.with(age: 2)\n  val b = Box { v: 1.5, n: 1 }\n  val c = b.with(v: 2.5)\n  println(q.name)\n}\n", &["check"]);
    assert!(ok, "{out}");
}

// BACKLOG item 369 - a user constructor used as a function value works, and a `priv` one named as a
// value outside its `impl` is E0214 like a direct call.
#[test]
fn constructors_as_function_values() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module K
type W = W(Text)
type Shape = Circle(Int) | Sq(Int, Int)
type Pt = { x: Int, y: Int }
fn show(w: W): Text = match w { W(t) => t }
fn area(s: Shape): Int = match s {
  Circle(r) => r
  Sq(a, b) => a * b
}
fn main(): Unit [io] = {
  val ws = List.map(["a", "b"], W)
  println(List.fold(List.map(ws, show), "", (a, b) => a ++ b))
  val cs = List.map([1, 2], Circle)
  println(intToText(List.fold(List.map(cs, area), 0, (a, b) => a + b)))
  val f = Circle
  println(intToText(area(f(7))))
  val g = Pt
  println(intToText(g(1, 2).y))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "ab\n3\n7\n2\n");

    let src = "module K\ntype Email = priv Email(Text)\nimpl Email {\n  fn new(raw: Text): Result<Email, Text> = if Text.len(raw) > 0 then Ok(Email(raw)) else Err(\"empty\")\n  fn mk(): List<Email> = List.map([\"a\"], Email)\n}\nfn bad1(): List<Email> = List.map([\"a\"], Email)\nfn bad2(): Unit = { val f = Email\n  () }\nfn ok(): Result<Email, Text> = Email.new(\"x\")\nfn main(): Unit [io] = println(\"x\")\n";
    let (ok, out) = certo(src, &["check"]);
    assert!(!ok && out.matches("E0214").count() == 2, "{out}");
}

// BACKLOG items 395-399 - DateTime keyword, strict ISO parsing, out-of-range instants and format
// specifiers, and number parsing.
fn run_program(src: &str) -> (bool, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    (r.status.success(), String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), String::from_utf8_lossy(&r.stderr).to_string())
}

#[test]
fn datetime_after_parses_and_dates_are_strict_and_total() {
    let (ok, out, err) = run_program(r####"module D
fn show(t: Timestamp?): Text = match t {
  Some(v) => "some"
  None => "none"
}
fn main(): Unit [io] = {
  val a = DateTime.fromUnix(100)
  val b = DateTime.fromUnix(200)
  println(boolToText(DateTime.after(b, a)))
  println(DateTime.toIso(DateTime.fromUnix(1750602600)))
  println(DateTime.format(DateTime.fromUnix(1750602600), "%A %d %B %Y %I:%M %p %j %z %Z"))
  println(DateTime.toIso(DateTime.fromUnix(-62167219200)))
  println(intToText(DateTime.year(DateTime.fromUnix(253402300799))))
  println(DateTime.toIso(DateTime.parseIso("2025-06-22T14:30:00+02:00")))
  println(DateTime.toIso(DateTime.parseIso("2025-06-22")))
  println(DateTime.toIso(DateTime.parseIso("2025-06-22T14:30:00.250-0530")))
  println(show(Timestamp.parse("2025-13-45T25:61:61Z")) ++ show(Timestamp.parse("2023-02-29T00:00:00Z")) ++ show(Timestamp.parse("2025-06-22T14:30:00Zjunk")) ++ show(Timestamp.parse("2025-6-2T1:2:3Z")))
  println(show(Timestamp.parse("2024-02-29T00:00:00Z")))
  println(Date.format(Date.of(2024, 2, 29), "%F"))
}
"####);
    assert!(ok, "{err}");
    assert_eq!(out, "true\n2025-06-22T14:30:00Z\nSunday 22 June 2025 02:30 PM 173 +0000 UTC\n0000-01-01T00:00:00Z\n9999\n2025-06-22T12:30:00Z\n2025-06-22T00:00:00Z\n2025-06-22T20:00:00Z\nnonenonenonenone\nsome\n2024-02-29\n");
    // An instant out of range, an impossible Date.of and an unknown specifier stop with a message
    // (and the output so far is kept) instead of crashing.
    for (body, msg) in [
        ("DateTime.toIso(DateTime.fromUnix(253402300800))", "out of range"),
        ("DateTime.format(DateTime.fromUnix(5), \"%Q\")", "unsupported format specifier %Q"),
        ("Date.format(Date.of(2023, 2, 29), \"%F\")", "not a valid calendar date"),
    ] {
        let (ok, out, err) = run_program(&format!("module D\nfn main(): Unit [io] = {{\n  println(\"before\")\n  println({body})\n}}\n"));
        assert!(!ok && out == "before\n" && err.contains(msg), "{body}: {out} {err}");
    }
}

#[test]
fn parse_int_and_parse_float_are_strict() {
    let (ok, out, err) = run_program(r####"module P
fn i(s: Text): Text = match parseInt(s) {
  Some(v) => intToText(v)
  None => "none"
}
fn f(s: Text): Text = match parseFloat(s) {
  Some(v) => floatToText(v)
  None => "none"
}
fn main(): Unit [io] = {
  println(i("42") ++ " " ++ i(" 5") ++ " " ++ i("5 ") ++ " " ++ i("-7") ++ " " ++ i("+3"))
  println(i("9223372036854775807") ++ " " ++ i("9223372036854775808") ++ " " ++ i("-9223372036854775808") ++ " " ++ i("-9223372036854775809"))
  println(i("") ++ " " ++ i("abc") ++ " " ++ i("1.5") ++ " " ++ i("0x10") ++ " " ++ i("- 1"))
  println(f("3.14") ++ " " ++ f(".5") ++ " " ++ f("5.") ++ " " ++ f("1e3") ++ " " ++ f("-2.5E-2") ++ " " ++ f(" 7 "))
  println(f("0x10") ++ " " ++ f("nan") ++ " " ++ f("inf") ++ " " ++ f("1e400") ++ " " ++ f("") ++ " " ++ f(".") ++ " " ++ f("1e") ++ " " ++ f("1.5x"))
  println(floatToText(sqrt(-1.0)))
}
"####);
    assert!(ok, "{err}");
    assert_eq!(out, "42 5 5 -7 3\n9223372036854775807 none -9223372036854775808 none\nnone none none none none\n3.14 0.5 5 1000 -0.025 7\nnone none none none none none none none\nnan\n");
}

// BACKLOG items 393/394 - `expect(x).toBe(y)` works for every value type, and test bodies are
// type-checked.
#[test]
fn expect_to_be_compares_any_value_and_test_bodies_are_checked() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module T
type Pt = { x: Int, y: Int }
type Shape = Circle(Int) | Dot
test "float" { expect(0.5 + 0.25).toBe(0.75) }
test "decimal" { expect(Decimal.add(d"10.00", d"12.00")).toBe(d"22.00") }
test "record" { expect(Pt { x: 1, y: 2 }).toBe(Pt { x: 1, y: 2 }) }
test "sum" { expect(Circle(2)).toBe(Circle(2)) }
test "option" { expect(Some(3)).toBe(Some(3)) }
test "option text" { expect(Some("a" ++ "b")).toBe(Some("ab")) }
test "option none" {
  val a: Int? = None
  expect(a).toBe(None)
}
test "option record" { expect(Some(Pt { x: 1, y: 2 })).toBe(Some(Pt { x: 1, y: 2 })) }
test "list" { expect([1, 2]).toBe([1, 2]) }
test "tuple" { expect((1, "a")).toBe((1, "a")) }
test "option differs" { assert(Some(3) != Some(4), "differs") }
"####).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("test").arg(&file).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && text.contains("All 11 tests passed"), "{text}");
    let (ok, out) = certo("module T\ntest \"t\" { val n: Int = \"hello\" }\nproperty \"p\" { undefinedFn(1) }\n", &["check"]);
    assert!(!ok && out.contains("E0200") && out.contains("E0206"), "{out}");
}

// BACKLOG item 385 - a Channel of Float or of a record carries its payload.
#[test]
fn channels_carry_floats_and_records() {
    let (ok, out, err) = run_program(r####"module C
type Order = { id: Int, total: Float }
fn producer(ch: Channel<Float>): Int [io] = {
  Channel.send(ch, 1.5)
  Channel.send(ch, 2.5)
  Channel.close(ch)
  0
}
fn main(): Unit [io, async] = {
  val ch: Channel<Float> = Channel.new(capacity: 4)
  val t = spawn producer(ch)
  var sum = 0.0
  var more = true
  while more {
    match Channel.receive(ch) {
      Some(v) => { sum = sum + v }
      None => { more = false }
    }
  }
  await t
  println(floatToText(sum))
  val oc: Channel<Order> = Channel.new(capacity: 2)
  Channel.send(oc, Order { id: 1, total: 9.5 })
  match Channel.tryReceive(oc) {
    Some(o) => println(floatToText(o.total) ++ " " ++ intToText(o.id))
    None => println("empty")
  }
  match Channel.tryReceive(oc) {
    Some(o) => println("unexpected")
    None => println("empty")
  }
}
"####);
    assert!(ok, "{err}");
    assert_eq!(out, "4\n9.5 1\nempty\n");
}

// BACKLOG items 413/414/415 - effects through function references, more seeded I/O functions, and
// the database effects.
#[test]
fn effects_follow_function_references_and_cover_the_database() {
    // A function named as a value carries its effect (item 413).
    let (ok, out) = certo("module E\nfn noisy(x: Int): Int [io] = { println(\"s\")\n  x }\nfn viaPipe(x: Int): Int [pure] = x |> noisy\nfn viaVal(x: Int): Int [pure] = { val f = noisy\n  f(x) }\nfn viaMap(xs: List<Int>): List<Int> [pure] = List.map(xs, noisy)\nfn fine(x: Int): Int [pure] = x + 1\nfn ok(x: Int): Int [io] = x |> noisy\n", &["check"]);
    assert!(!ok && out.matches("E0401").count() == 3, "{out}");
    // Naming an extern function outside `unsafe` is E0210 (item 413).
    let (ok, out) = certo("module F\nextern \"C\" {\n  fn llabs(n: Int): Int\n}\nfn a(n: Int): Int = n |> llabs\nfn b(xs: List<Int>): List<Int> = List.map(xs, llabs)\nfn ok(n: Int): Int = unsafe { n |> llabs }\n", &["check"]);
    assert!(!ok && out.matches("E0210").count() == 2, "{out}");
    // More functions are I/O (item 414).
    for call in ["Timestamp.now()", "Math.random()", "getEnv(\"HOME\")", "getCurrentDir()", "argCount()"] {
        let (ok, out) = certo(&format!("module P\nfn f(): Unit [pure] = {{\n  val x = {call}\n  ()\n}}\n"), &["check"]);
        assert!(!ok && out.contains("E0401"), "{call}: {out}");
    }
    // `db.read`, `db.write` and `db` (item 415).
    let (ok, out) = certo("module D\nimport Stdlib.Db\nfn r(conn: Int): List<List<Text?>> [db.read] = dbQuery(conn, \"SELECT 1\", [])\nfn w(conn: Int): Int [db.write] = dbExec(conn, \"DELETE FROM t\", [])\nfn rw(conn: Int): Int [db] = {\n  val x = dbQuery(conn, \"SELECT 1\", [])\n  dbExec(conn, \"DELETE FROM t\", [])\n}\nfn io(conn: Int): Int [io] = dbExec(conn, \"DELETE FROM t\", [])\nfn main(): Unit [io] = println(\"x\")\n", &["check"]);
    assert!(ok, "{out}");
    let (ok, out) = certo("module D\nimport Stdlib.Db\nfn bad(conn: Int): Int [db.read] = dbExec(conn, \"DELETE FROM t\", [])\nfn pure1(conn: Int): Int [pure] = dbExec(conn, \"DELETE FROM t\", [])\n", &["check"]);
    assert!(!ok && out.contains("E0400") && out.contains("db.write") && out.contains("E0401"), "{out}");
}
