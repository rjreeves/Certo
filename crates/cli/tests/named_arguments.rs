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
