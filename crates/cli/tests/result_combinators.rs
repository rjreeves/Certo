use std::{fs, process::Command};

// BACKLOG item 347 — `Result.getOrElse`/`recover`/`flatMap`/`mapErr`/`all`
// treated the pointer-sized Result payload slot as the raw value, but an
// `Ok(Int)` payload is heap-boxed and an `Ok(Float)` payload is stored as
// bits, so these combinators returned heap addresses / garbage doubles /
// failed to compile (struct payloads) / segfaulted (`mapErr` callbacks
// returning a non-pointer). Builds and runs real programs through the
// compiled `certo` binary.
fn build_and_run(src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

#[test]
fn result_combinators_handle_int_float_struct_and_enum_payloads() {
    let out = build_and_run(r####"module R

type P = { name: Text, qty: Int }
type AppError = NotFound | Invalid(Text)

fn v(n: Int): Result<Int, Text> = if n > 0 then Ok(n) else Err("neg")
fn half(x: Float): Result<Float, Text> = if x > 0.0 then Ok(x / 2.0) else Err("neg")
fn mk(n: Int): Result<P, Text> = if n > 0 then Ok(P { name: "item", qty: n }) else Err("none")
fn charge(n: Int): Result<Int, Text> = if n > 10 then Ok(n * 2) else Err("too small")
fn code(e: Text): Int = 42
fn toErr(e: Text): AppError = Invalid(e)
fn errName(e: AppError): Text = match e { NotFound => "nf", Invalid(m) => "inv:" ++ m }

fn main(): Unit [io] = {
  println(intToText(v(5).getOrElse(99)))
  println(intToText(v(0 - 1).getOrElse(99)))
  println(floatToText(half(9.0).getOrElse(9.5)))
  println(floatToText(half(0.0 - 1.0).getOrElse(9.5)))
  println(mk(3).getOrElse(P { name: "dflt", qty: 0 }).name)
  println(mk(0 - 1).getOrElse(P { name: "dflt", qty: 0 }).name)
  println(floatToText(half(0.0 - 1.0).recover((e) => 7.25)))
  println(floatToText(half(8.0).recover((e) => 7.25)))
  println(intToText(v(0 - 1).recover((e) => 7)))
  println(intToText(v(4).recover((e) => 7)))
  match v(20) |> flatMap(charge) { Ok(x) => println("ok " ++ intToText(x)), Err(m) => println("err " ++ m) }
  match v(3) |> flatMap(charge) { Ok(x) => println("ok " ++ intToText(x)), Err(m) => println("err " ++ m) }
  match v(0 - 1) |> flatMap(charge) { Ok(x) => println("ok " ++ intToText(x)), Err(m) => println("err " ++ m) }
  match v(20).flatMap((n) => Ok(n + 1)) { Ok(x) => println("ok " ++ intToText(x)), Err(m) => println("err " ++ m) }
  match v(0 - 1).mapErr(code) { Ok(x) => println("ok"), Err(c) => println("code " ++ intToText(c)) }
  match v(5).mapErr(code) { Ok(x) => println("ok " ++ intToText(x)), Err(c) => println("code " ++ intToText(c)) }
  match v(0 - 1).mapErr(toErr) { Ok(x) => println("ok"), Err(e) => println(errName(e)) }
  match half(0.0 - 1.0).mapErr((e) => e ++ "!") { Ok(x) => println("ok"), Err(e) => println(e) }
  match Result.all([v(1), v(2), v(3)]) { Ok(xs) => println(Text.join(List.map(xs, (n) => intToText(n)), ",")), Err(m) => println("err " ++ m) }
  match Result.all([v(1), v(0 - 2), v(3)]) { Ok(xs) => println("ok"), Err(m) => println("err " ++ m) }
  match Result.all([half(4.0), half(8.0)]) { Ok(xs) => println(Text.join(List.map(xs, (n) => floatToText(n)), ",")), Err(m) => println("err " ++ m) }
}
"####);
    assert_eq!(out, "5\n99\n4.5\n9.5\nitem\ndflt\n7.25\n4\n7\n4\nok 40\nerr too small\nerr neg\nok 21\ncode 42\nok 5\ninv:neg\nneg!\n1,2,3\nerr neg\n2,4\n");
}

#[test]
fn result_combinators_handle_text_bool_struct_payloads_and_chaining() {
    let out = build_and_run(r####"module R

type P = { name: Text, qty: Int }

fn t(n: Int): Result<Text, Int> = if n > 0 then Ok("t" ++ intToText(n)) else Err(n)
fn b(n: Int): Result<Bool, Text> = if n > 0 then Ok(true) else Err("no")
fn mk(n: Int): Result<P, Text> = if n > 0 then Ok(P { name: "p" ++ intToText(n), qty: n }) else Err("none")
fn safeDiv(a: Int, d: Int): Result<Int, Text> = if d == 0 then Err("div0") else Ok(a / d)

fn main(): Unit [io] = {
  println(t(2).getOrElse("zz"))
  println(t(0).getOrElse("zz"))
  println(if b(1).getOrElse(false) then "T" else "F")
  println(if b(0).getOrElse(true) then "T" else "F")
  match Result.all([t(1), t(2)]) { Ok(xs) => println(Text.join(xs, "+")), Err(e) => println("err") }
  match Result.all([b(1), b(2)]) { Ok(xs) => println(Text.join(List.map(xs, (x) => if x then "T" else "F"), "")), Err(e) => println("err " ++ e) }
  match Result.all([mk(1), mk(2)]) { Ok(xs) => println(Text.join(List.map(xs, (p) => p.name), "+")), Err(e) => println("err " ++ e) }
  val r: Result<Int, Text> = safeDiv(10, 2)
  println(intToText(r.getOrElse(0)))
  println(intToText(getOrElse(safeDiv(1, 0), 0 - 5)))
  println(intToText(safeDiv(20, 2).flatMap((x) => safeDiv(x, 5)).getOrElse(0 - 1)))
  println(intToText(safeDiv(20, 0).flatMap((x) => safeDiv(x, 5)).getOrElse(0 - 1)))
  println(intToText(safeDiv(20, 2).flatMap((x) => safeDiv(x, 0)).recover((e) => 0 - 9)))
  val xs = List.map([1, 2, 0], (d) => safeDiv(10, d).getOrElse(0))
  println(Text.join(List.map(xs, (n) => intToText(n)), ","))
}
"####);
    assert_eq!(out, "t2\nzz\nT\nT\nt1+t2\nTT\np1+p2\n5\n-5\n2\n-1\n-9\n10,5,0\n");
}
