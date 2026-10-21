use std::{fs, process::Command, time::{Duration, Instant}};

// BACKLOG item 398 - `Map.insert` in a loop was about cubic (4000 inserts took 34 s): neighbouring
// Int keys shared a bucket and every insert re-hashed the whole table.
#[test]
fn inserting_thousands_of_int_keys_in_a_loop_is_fast_and_correct() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, r####"module M
type P = { x: Int }
fn main(): Unit [io] = {
  var m = Map.empty()
  var j = 0
  while j < 4000 {
    m = Map.insert(m, j, j * 2)
    j = j + 1
  }
  println(intToText(Map.len(m)))
  println(intToText(Map.get(m, 3999) ?? -1))
  println(intToText(Map.get(m, 1234) ?? -1))
  println(intToText(Map.get(m, 4000) ?? -1))
  val m2 = Map.insert(m, 5, 99)
  println(intToText(Map.get(m, 5) ?? -1))
  println(intToText(Map.get(m2, 5) ?? -1))
  println(intToText(Map.len(m2)))
  var t = Map.empty()
  t = Map.insert(t, "a", 1)
  t = Map.insert(t, "b", 2)
  t = Map.insert(t, "a", 3)
  println(intToText(Map.len(t)))
  println(intToText(Map.get(t, "a") ?? -1))
}
"####).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let start = Instant::now();
    let r = Command::new(&exe).output().unwrap();
    assert!(r.status.success());
    assert!(start.elapsed() < Duration::from_secs(10), "took {:?}", start.elapsed());
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "4000\n7998\n2468\n-1\n10\n99\n4000\n2\n3\n");
}
