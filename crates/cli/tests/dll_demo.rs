use std::{fs, path::{Path, PathBuf}, process::Command};

// `dll-demo/` backs docs/DLL-INTEROP.md. Its sources went stale once (a new
// `[unsafe]` effect requirement, and extern names becoming verbatim), so the docs
// described programs that no longer compiled. Type-check every demo source so that
// can't happen unnoticed again.

fn cto_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            cto_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("cto") {
            out.push(path);
        }
    }
}

#[test]
fn every_dll_demo_source_type_checks() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dll-demo");
    let mut files = Vec::new();
    cto_files(&demo, &mut files);
    assert!(files.len() >= 10, "expected the demo sources under {}", demo.display());

    let mut failures = Vec::new();
    for file in &files {
        let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("check").arg(file).output().unwrap();
        if !out.status.success() {
            let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            let first = text.lines().find(|l| l.contains("error")).unwrap_or("").to_string();
            failures.push(format!("{}: {}", file.strip_prefix(&demo).unwrap().display(), first));
        }
    }
    assert!(failures.is_empty(), "dll-demo sources that no longer compile:\n{}", failures.join("\n"));
}
