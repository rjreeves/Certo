use std::{fs, process::Command};

// BACKLOG item 360 - `x => body` is a lambda with one bare parameter, and `Result.map` exists.
fn run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n")
}

#[test]
fn bare_lambdas_and_result_map() {
    let out = run(r####"module L
fn half(n: Int): Result<Int, Text> = if n % 2 == 0 then Ok(n / 2) else Err("odd")
fn show(r: Result<Int, Text>): Text = match r {
  Ok(v) => "ok " ++ intToText(v)
  Err(e) => "err " ++ e
}
fn showF(r: Result<Float, Text>): Text = match r {
  Ok(v) => "okf " ++ floatToText(v)
  Err(e) => "err " ++ e
}
fn showT(r: Result<Text, Text>): Text = match r {
  Ok(v) => "okt " ++ v
  Err(e) => "err " ++ e
}
fn sign(b: Bool, n: Int): Text = match n {
  x if b => "guarded"
  _ => "plain"
}
fn main(): Unit [io] = {
  println(show(half(8).map(v => v + 1)))
  println(show(half(7).map(v => v + 1)))
  println(showF(half(8).map(v => intToFloat(v) / 2.0)))
  println(showT(half(8).map(v => "n=" ++ intToText(v))))
  println(show(half(8).flatMap(v => half(v))))
  println(intToText(half(7).recover(e => 0)))
  println(intToText(List.len(List.filter([1, 2, 3, 4], n => n > 2))))
  println(show(Result.map(half(4), v => v * 10)))
  println(sign(true, 1))
  println(sign(false, 1))
}
"####);
    assert_eq!(out, "ok 5\nerr odd\nokf 2\nokt n=4\nok 2\n0\n2\nok 20\nguarded\nplain\n");
}
