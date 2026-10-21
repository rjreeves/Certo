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

// BACKLOG item 440 - only `pub` functions, values and variables of an imported file are visible
// to its importer; the private ones still work inside their own file.
#[test]
fn a_private_function_of_an_imported_file_is_not_callable_from_the_importer() {
    let dir = project();
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib\nfn main(): Unit [io] = {\n  println(intToText(helper(mk(1))))\n}\n");
    let (ok, _, err) = build_run(dir.path(), "main.cto");
    assert!(!ok, "calling a private function of another file must not compile");
    assert!(err.contains("helper"), "{err}");
}

#[test]
fn a_private_helper_still_works_inside_its_own_file_and_names_can_be_reused() {
    let dir = project();
    // `helper` is private to Lib; the importer may define its own `helper`.
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib\nfn helper(): Int = 7\nfn main(): Unit [io] = {\n  println(intToText(other() + helper()))\n}\n");
    let (ok, out, err) = build_run(dir.path(), "main.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "108\n");
}

#[test]
fn importing_a_private_name_by_name_is_an_error() {
    let dir = project();
    write(dir.path(), "main.cto", "module Main\nimport MyApp.Lib.{ helper }\nfn main(): Unit [io] = println(\"x\")\n");
    let (ok, _, err) = build_run(dir.path(), "main.cto");
    assert!(!ok);
    assert!(err.contains("E0105") && err.contains("`helper` is private in module `MyApp.Lib`"), "{err}");
}

// BACKLOG item 443 - types, traits and variants follow `pub` too, and a file's own imports reach
// its importer only through `pub import`.
fn visibility_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "App/Models.cto", "module App.Models\npub type Order = { total: Int }\ntype Secret = { code: Int }\npub type Kind = Big | Small\npub fn mk(n: Int): Order = Order { total: n }\nfn hidden(): Secret = Secret { code: 1 }\npub fn reveal(): Int = hidden().code\n");
    write(dir.path(), "App/Lib.cto", "module App.Lib\nimport App.Models\npub fn twice(o: Order): Int = o.total * 2\n");
    write(dir.path(), "App/Pub.cto", "module App.Pub\npub import App.Models\npub fn id(n: Int): Int = n\n");
    dir
}

#[test]
fn private_and_unimported_types_are_errors() {
    let dir = visibility_project();
    // A type reached only through a file's own (non-`pub`) import.
    write(dir.path(), "m1.cto", "module Main\nimport App.Lib\nfn main(): Unit [io] = println(intToText(twice(Order { total: 2 })))\n");
    let (ok, _, err) = build_run(dir.path(), "m1.cto");
    assert!(!ok && err.contains("E0235") && err.contains("`Order` is declared in module `App.Models`, which this file does not import"), "{err}");
    // A private type.
    write(dir.path(), "m3.cto", "module Main\nimport App.Models\nfn f(s: Secret): Int = s.code\nfn main(): Unit [io] = println(\"x\")\n");
    let (ok, _, err) = build_run(dir.path(), "m3.cto");
    assert!(!ok && err.contains("E0235") && err.contains("`Secret` is private to module `App.Models`"), "{err}");
}

#[test]
fn public_types_direct_pub_import_and_both_imports_work() {
    let dir = visibility_project();
    write(dir.path(), "m2.cto", "module Main\nimport App.Models\nfn main(): Unit [io] = println(intToText(reveal() + mk(3).total))\n");
    let (ok, out, err) = build_run(dir.path(), "m2.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "4\n");
    write(dir.path(), "m4.cto", "module Main\nimport App.Pub\nfn main(): Unit [io] = println(intToText(mk(Order { total: 3 }.total).total))\n");
    let (ok, _, err) = build_run(dir.path(), "m4.cto");
    assert!(ok, "pub import passes the types on: {err}");
    write(dir.path(), "m5.cto", "module Main\nimport App.Models\nimport App.Lib\nfn main(): Unit [io] = println(intToText(twice(mk(3))))\n");
    let (ok, out, err) = build_run(dir.path(), "m5.cto");
    assert!(ok, "{err}");
    assert_eq!(out, "6\n");
}
