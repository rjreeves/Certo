use std::{fs, process::Command};

// BACKLOG item 355 - `.age` is the Timestamp operator only for a Timestamp receiver; a record with
// a field called `age` reads that field.
#[test]
fn a_record_field_named_age_can_be_read() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, r####"module P
type P = { name: Text, age: Int }
impl P {
  fn older(self): Int = self.age + 1
}
fn main(): Unit [io] = {
  val p = P { name: "a", age: 41 }
  println(intToText(p.age))
  println(intToText(p.older()))
  val q = p.with(age: 7)
  println(intToText(q.age))
  val ts = Timestamp.now()
  val d = ts.age
  println("ts ok")
}
"####).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "build failed: {}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert!(r.status.success());
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "41\n42\n7\nts ok\n");
}
