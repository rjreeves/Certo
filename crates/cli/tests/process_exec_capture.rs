use std::{fs, process::Command};

// BACKLOG item 427 — `Process.exec` was changed to start its shell with
// CREATE_NO_WINDOW (instead of `system()`, which flashed a console window
// from a GUI-subsystem parent). This pins that the captured behaviour is
// unchanged: stdout/stderr are captured separately and the exit code is the
// command's. Builds and runs a real program through the compiled binary.
fn build_and_run(src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

#[test]
fn process_exec_still_captures_stdout_and_the_exit_code() {
    let (prog, args, want_out) = if cfg!(windows) {
        ("where.exe", "\"cmd.exe\"", "cmd.exe")
    } else {
        ("echo", "\"hello\"", "hello")
    };
    let out = build_and_run(&format!(r####"module P
fn main(): Unit [io] = {{
  val r = Process.exec("{prog}", [{args}])
  println("exit=" ++ intToText(ProcessResult.exitCode(r)))
  println("out=" ++ Text.trim(ProcessResult.stdout(r)))
  val m = Process.exec("certo-no-such-program-xyz", [])
  println("missing=" ++ intToText(ProcessResult.exitCode(m)))
}}
"####));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "exit=0");
    assert!(lines[1].starts_with("out=") && lines[1].to_lowercase().contains(want_out), "{}", lines[1]);
    assert_ne!(lines[2], "missing=0", "a nonexistent command must not report success");
}
