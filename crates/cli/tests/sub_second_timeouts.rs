use std::{fs, process::Command};

// BACKLOG item 383 - `withTimeout` / `parallel(timeout:)` / `every` turned a Duration into
// milliseconds through `toSeconds`, so a sub-second Duration became 0 (an immediate timeout).
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
fn sub_second_and_fractional_second_timeouts_keep_their_milliseconds() {
    let out = run(r####"module T
async fn quick(): Int [io] = {
  sleep(100)
  42
}
async fn slow(): Int [io] = {
  sleep(1500)
  7
}
fn show(r: Int?): Unit [io] = match r {
  Some(v) => println(intToText(v))
  None => println("timeout")
}
async fn main(): Unit [io] = {
  show(withTimeout(Duration.milliseconds(900)) { quick() })
  show(withTimeout(Duration.milliseconds(1999)) { quick() })
  show(withTimeout(Duration.milliseconds(200)) { slow() })
  val (a, b) = await parallel(timeout: Duration.milliseconds(900)) { quick(), quick() }
  println(intToText(a + b))
  println(intToText(Duration.toMilliseconds(Duration.seconds(2))))
}
"####);
    assert_eq!(out, "42\n42\ntimeout\n84\n2000\n");
}
