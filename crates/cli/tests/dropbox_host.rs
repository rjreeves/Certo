use std::{path::PathBuf, process::Command};

#[test]
fn dropbox_plugin_builds_as_a_long_running_host() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/dropbox_host.cto");
    let directory = tempfile::tempdir().expect("temporary build directory");
    let executable = directory.path().join(if cfg!(windows) {
        "dropbox-host.exe"
    } else {
        "dropbox-host"
    });
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("build Dropbox host example");

    assert!(
        output.status.success(),
        "certo failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(executable.exists(), "Dropbox host executable was not produced");
}
