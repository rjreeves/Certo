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
