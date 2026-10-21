use std::{fs, process::Command};

// BACKLOG item 351 - f-strings: `{{` / `}}` / `\{` / `\}` write a literal brace, and an
// interpolation may contain a string literal. (Backslash escapes already worked.)
#[test]
fn fstring_literal_braces_and_string_literals_inside_interpolations() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, r####"module F
type Money = { amount: Int, cur: Text }
fn show(m: Money): Text = intToText(m.amount) ++ m.cur
fn main(): Unit [io] = {
  val n = 5
  println(f"a\tb {n}\nline2")
  println(f"set {{n}} = {n}")
  println(f"esc \{n\} = {n}")
  println(f"{show(Money { amount: 10, cur: "USD" })} paid")
  println(f"{"quoted {x}"} and {n}")
  println(f"close }} brace")
  println(f"héllo {n} ünï")
}
"####).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"),
        "a\tb 5\nline2\nset {n} = 5\nesc {n} = 5\n10USD paid\nquoted {x} and 5\nclose } brace\nhéllo 5 ünï\n");
}

#[test]
fn the_formatter_keeps_literal_braces_and_inner_strings() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, "module F\nfn main(): Unit [io] = println(f\"set {{n}} and {\"q\"} \\{x\\}\")\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("fmt").arg(&file).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("f\"set {{n}} and {\"q\"} {{x}}\""), "{text}");
}
