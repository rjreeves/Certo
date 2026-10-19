use std::{fs, path::Path, process::Command};

// BACKLOG items 403/404 — `certo fmt` used to drop grouping parentheses
// (`(a + b) * c` -> `a + b * c`), print f-string interpolation as `${x}`,
// double-escape f-string text, and strip the parentheses from lambda
// parameters, so a formatted program either computed a different answer or
// stopped parsing. Each test builds and runs a real program, formats it in
// place with the real `certo fmt`, builds and runs it again, and requires the
// two runs to agree — the property that actually matters, rather than the
// shape of the formatted text.

fn build_and_run(file: &Path, exe_name: &str) -> String {
    let exe = file.with_file_name(if cfg!(windows) { format!("{exe_name}.exe") } else { exe_name.to_string() });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(file).arg("-o").arg(&exe).output().unwrap();
    assert!(
        build.status.success(),
        "build of {} failed: {}{}",
        file.display(),
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

/// Runs `src`, formats it in place with `certo fmt`, runs it again.
/// Returns `(output before, output after, formatted source)`.
fn run_before_and_after_fmt(src: &str) -> (String, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();

    let before = build_and_run(&file, "before");

    let fmt = Command::new(env!("CARGO_BIN_EXE_certo")).arg("fmt").arg(&file).output().unwrap();
    assert!(fmt.status.success(), "certo fmt failed: {}", String::from_utf8_lossy(&fmt.stderr));
    let formatted = fs::read_to_string(&file).unwrap().replace("\r\n", "\n");

    let after = build_and_run(&file, "after");
    (before, after, formatted)
}

#[test]
fn grouping_parentheses_survive_formatting() {
    // Every function here gives a different answer if its parentheses are
    // dropped: `(a + b) * 2`, `a - (b - c)`, `(a ** b) ** 2` (`**` is
    // right-associative), `-(a + 1)`, `(p or q) and r`, `not (p and q)` and a
    // parenthesised `if`.
    let (before, after, formatted) = run_before_and_after_fmt(r#"module F
import Stdlib.Core

fn calc(a: Int, b: Int): Int = (a + b) * 2
fn mix(a: Int, b: Int, c: Int): Int = a - (b - c) + (a + b) * c / (a - c)
fn pw(a: Int, b: Int): Int = (a ** b) ** 2
fn neg(a: Int): Int = -(a + 1)
fn logic1(p: Bool, q: Bool, r: Bool): Bool = (p or q) and r
fn logic2(p: Bool, q: Bool): Bool = not (p and q)
fn pick(p: Bool, a: Int): Int = (if p then a else 0) + 100

fn main(): Unit [io] = {
  println(intToText(calc(1, 2)))
  println(intToText(mix(7, 3, 2)))
  println(intToText(pw(2, 3)))
  println(intToText(neg(4)))
  println(boolToText(logic1(true, false, false)))
  println(boolToText(logic2(true, false)))
  println(intToText(pick(true, 5)))
}
"#);
    assert_eq!(before, "6\n10\n64\n-5\nfalse\ntrue\n105\n");
    assert_eq!(after, before, "formatting changed what the program prints\n--- formatted source ---\n{formatted}");
    assert!(formatted.contains("(a + b) * 2"), "{formatted}");
}

#[test]
fn fstring_interpolation_and_text_survive_formatting() {
    let (before, after, formatted) = run_before_and_after_fmt(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
  val x = 5
  println(f"x is {x}")
  println(f"tab[\t] quote[\"] {x}")
}
"#);
    assert!(before.starts_with("x is 5\n"), "{before}");
    // Item 431: escapes inside an f-string are processed like any string's.
    assert!(before.contains("tab[\t] quote[\"] 5"), "escapes must be processed: {before:?}");
    assert_eq!(after, before, "formatting changed what the program prints\n--- formatted source ---\n{formatted}");
    assert!(!formatted.contains("${"), "interpolation must stay `{{x}}`, not `${{x}}`\n{formatted}");
    // The text around the interpolation is kept exactly as written, escapes included.
    assert!(formatted.contains(r#"f"tab[\t] quote[\"] {x}""#), "{formatted}");
}

#[test]
fn lambda_parameter_parentheses_survive_formatting() {
    // The parser has no bare `x => ...` form, so a formatted file that dropped
    // the parentheses stopped parsing at all.
    let (before, after, formatted) = run_before_and_after_fmt(r#"module F
import Stdlib.Core
import Stdlib.Collections.{ List }

fn main(): Unit [io] = {
  val doubled = List.map([1, 2, 3], (x) => x * 2)
  val total = List.fold(doubled, 0, (acc, x) => acc + x)
  println(intToText(total))
}
"#);
    assert_eq!(before, "12\n");
    assert_eq!(after, before, "formatting changed what the program prints\n--- formatted source ---\n{formatted}");
    assert!(formatted.contains("(x) => x * 2"), "{formatted}");
}
