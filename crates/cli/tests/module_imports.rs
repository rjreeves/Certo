use std::{fs, path::Path, process::Command};

// BACKLOG item 359 - an imported file's own imports are resolved, each file is merged once, a named
// import keeps the helpers its functions need (under hidden names), and a file's `module` line
// must mirror the path it is imported by.
fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn build_run(dir: &Path, main: &str) -> (bool, String, String) {
    let exe = dir.join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(dir.join(main)).arg("-o").arg(&exe).output().unwrap();
    let err = String::from_utf8_lossy(&b.stderr).to_string();
    if !b.status.success() { return (false, String::new(), err); }
    let r = Command::new(&exe).output().unwrap();
    (true, String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), err)
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "MyApp/Models/Order.cto", "module MyApp.Models.Order\npub type Order = { total: Int }\npub fn mk(n: Int): Order = Order { total: n }\n");
    write(dir.path(), "MyApp/Lib.cto", "module MyApp.Lib\nimport MyApp.Models.Order\nfn helper(o: Order): Int = o.total + 100\npub fn other(): Int = helper(mk(1))\npub fn unused(): Int = 0\n");
    dir
}

#[test]
fn an_imported_files_own_imports_are_resolved() {
    let dir = project();
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib.{ other }\nfn main(): Unit [io] = {\n  println(intToText(other()))\n}\n");
    let (ok, out, err) = build_run(dir.path(), "main.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "101\n");
}

#[test]
fn a_file_imported_twice_is_merged_once() {
    let dir = project();
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib\nimport MyApp.Models.Order\nfn main(): Unit [io] = {\n  println(intToText(other() + mk(5).total))\n}\n");
    let (ok, out, err) = build_run(dir.path(), "main.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "106\n");
}

#[test]
fn an_import_cycle_does_not_loop() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "cyc_a.cto", "module cyc_a\nimport cyc_b\npub fn a(): Int = 1\n");
    write(dir.path(), "cyc_b.cto", "module cyc_b\nimport cyc_a\npub fn b(): Int = 2\n");
    write(dir.path(), "main.cto", "module Main\nimport cyc_a\nfn main(): Unit [io] = { println(intToText(a() + b())) }\n");
    let (ok, out, err) = build_run(dir.path(), "main.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "3\n");
}

#[test]
fn a_named_import_does_not_expose_the_unlisted_helpers() {
    let dir = project();
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib.{ other }\nfn main(): Unit [io] = {\n  println(intToText(unused()))\n}\n");
    let (ok, _out, err) = build_run(dir.path(), "main.cto");
    assert!(!ok);
    assert!(err.contains("unused"), "{err}");
}

#[test]
fn a_module_line_that_does_not_match_the_import_path_is_error_e0104() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Wrong.cto", "module Totally.Different\npub fn w(): Int = 1\n");
    write(dir.path(), "main.cto", "module Main\nimport Wrong\nfn main(): Unit [io] = { println(intToText(w())) }\n");
    let (ok, _out, err) = build_run(dir.path(), "main.cto");
    assert!(!ok);
    assert!(err.contains("E0104") && err.contains("Totally.Different"), "{err}");
}
