use std::{fs, process::Command};

// BACKLOG items 416/417/420 - validateWithDb codegen, after/overrides gating shared by
// validate and validateAll, and duplicate / reserved / self-overriding rule names.
fn certo(src: &str, args: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).args(args).arg(&file).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n") + &String::from_utf8_lossy(&out.stderr))
}

#[test]
fn validate_with_db_builds_and_runs() {
    let (ok, out) = certo(r####"module B
type C = { credit: Int }
type T = { cid: Int, total: Int }
type VE = | Over
fn loadC(id: Int): C = C { credit: 100 + id }
validator V for T errors VE {
  context { customer: C loaded by loadC(t.cid) }
  rule within { require t.total <= customer.credit else VE.Over }
}
fn main(): Unit [io] = {
  println(match V.validateWithDb(T { cid: 1, total: 50 }) { Ok(_) => "ok", Err(_) => "over" })
  println(match V.validateWithDb(T { cid: 1, total: 500 }) { Ok(_) => "ok", Err(_) => "over" })
}
"####, &["run"]);
    assert!(ok && out.trim() == "ok\nover", "{out}");
}

#[test]
fn skipped_prerequisites_and_overrides_agree_between_validate_and_validate_all() {
    let (ok, out) = certo(r####"module A
type VE = | A | B | C | X | Y
type E = { a: Int, b: Int, c: Int, admin: Bool }
validator V for E errors VE {
  rule ra { require e.a > 0 else VE.A }
  rule rb { after ra require e.b > 0 else VE.B }
  rule rc { after rb require e.c > 0 else VE.C }
}
validator W for E errors VE {
  rule rx { require e.a > 0 else VE.X }
  rule ry { after rx require e.b > 0 else VE.Y }
  rule ro { priority 1 overrides rx require e.admin else VE.X }
}
fn main(): Unit [io] = {
  println(intToText(List.len(V.validateAll(E { a: 0, b: 1, c: 0, admin: false }))))
  println(intToText(List.len(W.validateAll(E { a: 0, b: 0, c: 0, admin: true }))))
  println(match W.validate(E { a: 0, b: 0, c: 0, admin: true }) { Ok(_) => "ok", Err(_) => "failed" })
  println(intToText(List.len(W.validateAll(E { a: 0, b: 0, c: 0, admin: false }))))
}
"####, &["run"]);
    assert!(ok && out.trim() == "1\n0\nok\n1", "{out}");
}

#[test]
fn bad_rule_names_are_diagnosed() {
    let (ok, out) = certo(r####"module A
type VE = | A
type E = { a: Int }
validator V for E errors VE {
  rule r { require e.a > 0 else VE.A }
  rule r { require e.a > 1 else VE.A }
  rule validate { require e.a > 2 else VE.A }
  rule s { overrides s require e.a > 3 else VE.A }
}
"####, &["check"]);
    assert!(!ok && out.contains("E0712") && out.contains("E0713") && out.contains("E0714"), "{out}");
}
