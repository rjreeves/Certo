use std::process::Command;

// BACKLOG item 409 - every `certo new` template scaffolds a project that parses and checks, a
// library builds with a bare `certo build`, and an unknown template is an error.
fn certo(dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).args(args).current_dir(dir).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string() + &String::from_utf8_lossy(&out.stdout))
}

#[test]
fn every_template_checks_and_a_library_builds() {
    let dir = tempfile::tempdir().unwrap();
    for t in ["default", "api", "lib", "cli", "fullstack"] {
        let name = format!("p_{t}");
        let (ok, out) = certo(dir.path(), &["new", &name, "--template", t]);
        assert!(ok, "{t}: {out}");
        let (ok, out) = certo(&dir.path().join(&name), &["check", "src/main.cto"]);
        assert!(ok && !out.contains("error"), "{t}: {out}");
    }
    let (ok, out) = certo(&dir.path().join("p_lib"), &["build"]);
    assert!(ok, "{out}");
    let (ok, out) = certo(&dir.path().join("p_default"), &["build"]);
    assert!(ok, "{out}");
    // A new project is already formatted (the formatter keeps a block body on the signature line).
    for t in ["default", "api", "lib", "cli"] {
        let (ok, out) = certo(&dir.path().join(format!("p_{t}")), &["fmt", "--check"]);
        assert!(ok && !out.contains("would reformat"), "{t}: {out}");
    }
    let gi = std::fs::read_to_string(dir.path().join("p_default").join(".gitignore")).unwrap();
    assert!(gi.lines().any(|l| l == "dist/") && gi.lines().any(|l| l == ".cto/"), "{gi}");
}

#[test]
fn an_unknown_template_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = certo(dir.path(), &["new", "x", "--template", "bogus"]);
    assert!(!ok && out.contains("unknown template 'bogus'"), "{out}");
    assert!(!dir.path().join("x").exists());
}

// BACKLOG item 407 - `check`, `run`, `test`, `fmt` and `lint` work without file arguments inside a
// project, `test` takes directories, `ruleTest`/`validatorTest` need no `context:` for a validator
// without one, and `--filter` matches a validator's name.
#[test]
fn commands_without_files_use_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, out) = certo(dir.path(), &["new", "proj"]);
    assert!(ok, "{out}");
    let proj = dir.path().join("proj");
    std::fs::create_dir_all(proj.join("tests").join("unit")).unwrap();
    std::fs::write(proj.join("tests").join("unit").join("add.cto"),
        "module T\nfn add(a: Int, b: Int): Int = a + b\ntest \"adds\" {\n  expect(add(1, 2)).toBe(3)\n}\n").unwrap();
    std::fs::write(proj.join("tests").join("unit").join("rules.cto"),
        "module V\ntype Order = { lines: Int }\ntype OrderError = NoLines | TooBig\nvalidator Submit for Order errors OrderError {\n  rule has_lines {\n    require order.lines > 0\n    else OrderError.NoLines\n  }\n}\nruleTest Submit.has_lines \"empty fails\" {\n  entity: Order { lines: 0 }\n  expect: fail\n}\nvalidatorTest Submit \"good passes\" {\n  entity: Order { lines: 2 }\n  expect: pass\n}\n").unwrap();
    let (ok, out) = certo(&proj, &["check"]);
    assert!(ok && out.contains("ok"), "{out}");
    let (ok, out) = certo(&proj, &["run"]);
    assert!(ok && out.contains("Hello from proj!"), "{out}");
    let (ok, out) = certo(&proj, &["test"]);
    assert!(ok && out.contains("adds") && out.contains("empty fails") && out.contains("good passes") && !out.contains("FAIL"), "{out}");
    let (ok, out) = certo(&proj, &["test", "tests"]);
    assert!(ok && out.contains("adds") && out.contains("good passes"), "{out}");
    let (ok, out) = certo(&proj, &["test", "--filter", "Submit"]);
    assert!(ok && out.contains("Submit.has_lines: empty fails") && !out.contains("adds"), "{out}");
    let (ok, out) = certo(&proj, &["lint"]);
    assert!(ok, "{out}");
}
