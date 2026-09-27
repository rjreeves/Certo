use std::{fs, process::Command};

fn write_hello(root: &std::path::Path) -> std::path::PathBuf {
    let src = root.join("main.cto");
    fs::write(&src, "module Repro\nfn main(): Unit = println(\"hi\")\n").unwrap();
    src
}

// BACKLOG item 324 — `certo build f.cto -o out.exe` then `certo build
// f.cto -o out.exe --emit-c` used to silently overwrite the working binary
// with C source text, printing the identical "wrote out.exe" success
// message either way. `--emit-c` combined with an explicit `-o` that
// doesn't end in `.c` must now be refused, leaving the existing file
// completely untouched, rather than guessing.
#[test]
fn emit_c_with_non_c_output_path_is_rejected_and_leaves_existing_file_untouched() {
    let root = tempfile::tempdir().unwrap();
    let src = write_hello(root.path());
    let out_path = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });

    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&src).arg("-o").arg(&out_path)
        .output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let original_bytes = fs::read(&out_path).unwrap();
    assert!(!original_bytes.is_empty());

    let emit_c = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&src).arg("-o").arg(&out_path).arg("--emit-c")
        .output().unwrap();
    assert!(!emit_c.status.success(), "--emit-c with a non-.c -o path must be refused, not silently accepted");
    let stderr = String::from_utf8_lossy(&emit_c.stderr);
    assert!(stderr.contains("--emit-c"), "expected an error message naming --emit-c, got: {stderr}");

    let bytes_after = fs::read(&out_path).unwrap();
    assert_eq!(original_bytes, bytes_after, "the existing binary must survive completely untouched");
}

#[test]
fn emit_c_with_explicit_c_output_path_still_works() {
    let root = tempfile::tempdir().unwrap();
    let src = write_hello(root.path());
    let out_path = root.path().join("out.c");

    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&src).arg("-o").arg(&out_path).arg("--emit-c")
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let emitted = fs::read_to_string(&out_path).unwrap();
    assert!(emitted.contains("#include"), "expected real C source to be written");
}

#[test]
fn emit_c_with_no_output_path_still_defaults_to_stem_dot_c() {
    let root = tempfile::tempdir().unwrap();
    let src = write_hello(root.path());

    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&src).arg("--emit-c")
        .current_dir(root.path())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let emitted = fs::read_to_string(root.path().join("main.c")).unwrap();
    assert!(emitted.contains("#include"), "expected real C source to be written");
}
