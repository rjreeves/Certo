use std::{fs, process::Command};

// BACKLOG item 353 - a record literal must give every field that is not optional.
fn check(src: &str) -> (bool, String) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("check").arg(&file).output().unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

#[test]
fn omitting_a_required_field_is_error_e0226() {
    let (ok, text) = check("module P\ntype User = { id: Int, name: Text }\nfn main(): Unit [io] = {\n  val u = User { id: 1 }\n  println(u.name)\n}\n");
    assert!(!ok);
    assert!(text.contains("E0226") && text.contains("`name`"), "{text}");
}

#[test]
fn optional_computed_and_copy_update_forms_are_still_accepted() {
    let (ok, text) = check(r####"module P
type User = { id: Int, name: Text, nick?: Text, note: Text? }
fn main(): Unit [io] = {
  val u = User { id: 1, name: "a" }
  val v = u.with(id: 2)
  println(v.name)
}
"####);
    assert!(ok, "{text}");
}
