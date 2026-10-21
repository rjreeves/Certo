use std::{fs, process::Command};

// BACKLOG item 384 - `spawn` returns a typed task handle (`Task<R>`), so it can be stored in a
// List, passed to and returned from functions, and awaited later for any result type.
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
fn task_handles_can_be_stored_passed_returned_and_awaited() {
    let out = run(r####"module T
fn heavy(n: Int): Int = n * n
async fn collect(t: Task<Int>): Int = await t
fn mk(n: Int): Task<Int> = spawn heavy(n)
type Ev = { name: Text }
fn mkEv(n: Int): Ev = Ev { name: "e" ++ intToText(n) }
fn okr(n: Int): Result<Int, Text> = if n > 0 then Ok(n) else Err("neg")
fn opt(n: Int): Int? = Some(n)
async fn main(): Unit [io] = {
  val t = spawn heavy(3)
  println(intToText(await collect(t)))
  var hs: List<Task<Int>> = []
  hs = List.push(hs, spawn heavy(4))
  hs = List.push(hs, spawn heavy(5))
  var total = 0
  List.forEach(hs, (h: Task<Int>) => { total = total + await h })
  println(intToText(total))
  val ts = List.map([1, 2, 3], (n) => spawn heavy(n))
  println(intToText(List.fold(ts, 0, (a, h: Task<Int>) => a + await h)))
  println(intToText(await mk(6)))
  val r = spawn okr(7)
  match await r {
    Ok(v) => println(intToText(v))
    Err(e) => println(e)
  }
  val e = spawn mkEv(2)
  println((await e).name)
  val o = spawn opt(9)
  match await o {
    Some(v) => println(intToText(v))
    None => println("none")
  }
}
"####);
    assert_eq!(out, "9
41
14
36
7
e2
9
");
}

#[test]
fn a_user_type_named_task_is_unaffected() {
    let out = run(r####"module T
type Task = { id: Int }
fn show(t: Task): Int = t.id
fn main(): Unit [io] = println(intToText(show(Task { id: 5 })))
"####);
    assert_eq!(out, "5
");
}
