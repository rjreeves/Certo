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

// BACKLOG item 428 — `Process.execWithInput` and `Process.lines` stopped using _popen (which
// flashed a console window from a GUI-subsystem program) and now run a hidden child with a
// pipe. These pin that what they deliver is unchanged: the input reaches the child's stdin,
// every output line reaches the handler, exit codes survive, and large input/output does not
// deadlock on a full pipe.
fn run_program(src: &str) -> String {
    build_and_run(src)
}

#[test]
fn process_exec_with_input_delivers_stdin_and_captures_the_output() {
    // `findstr a` / `grep a` print the input lines that contain an "a".
    let prog = if cfg!(windows) { "findstr" } else { "grep" };
    let out = run_program(&format!(r####"module P
fn main(): Unit [io] = {{
  val r = Process.execWithInput("{prog}", ["a"], "abc\nxyz\nbanana\n")
  println("exit=" ++ intToText(ProcessResult.exitCode(r)))
  println("out=" ++ Text.replace(Text.trim(ProcessResult.stdout(r)), "\r", ""))
  val none = Process.execWithInput("{prog}", ["q"], "abc\n")
  println("none=" ++ intToText(ProcessResult.exitCode(none)))
}}
"####));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "exit=0");
    // The two matching input lines come back, one per output line.
    assert_eq!(lines[1], "out=abc");
    assert_eq!(lines[2], "banana");
    assert_ne!(*lines.last().unwrap(), "none=0", "no match is a non-zero exit");
}

#[test]
fn process_exec_with_input_handles_input_larger_than_a_pipe_buffer() {
    // 20,000 lines is ~260 KB, well past a pipe's buffer: writing it must not deadlock.
    let prog = if cfg!(windows) { "findstr" } else { "grep" };
    let out = run_program(&format!(r####"module P
fn build(n: Int, acc: Text): Text = if n == 0 then acc else build(n - 1, acc ++ "line with an a\n")

fn main(): Unit [io] = {{
  val r = Process.execWithInput("{prog}", ["a"], build(20000, ""))
  val out = ProcessResult.stdout(r)
  println("exit=" ++ intToText(ProcessResult.exitCode(r)))
  println("count=" ++ intToText(List.len(Text.split(Text.trim(out), "\n"))))
}}
"####));
    assert_eq!(out, "exit=0\ncount=20000\n");
}

/// Builds and runs `src` with `dir` as its working directory, so a test can name a data file
/// relatively (no Windows path to escape inside a Certo string).
fn run_program_in(dir: &std::path::Path, src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&exe).current_dir(dir).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n")
}

/// The command (and its Certo argument list) that prints `file` line by line: `cat` on POSIX,
/// `cmd /c type` on Windows — `Process.lines` quotes every argument, and `findstr` does not
/// accept a quoted switch.
fn print_file_command(file: &str) -> (&'static str, String) {
    if cfg!(windows) {
        ("cmd", format!("\"/c\", \"type {file}\""))
    } else {
        ("cat", format!("\"{file}\""))
    }
}

#[test]
fn process_lines_delivers_every_line_and_the_exit_code() {
    // The program streams a known file back line by line.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("data.txt"), "alpha\nbeta\ngamma\n").unwrap();
    let (prog, args) = print_file_command("data.txt");
    let out = run_program_in(dir.path(), &format!(r####"module P
var seen = ""
var count = 0

fn collect(line: Text): Unit = {{
  seen = seen ++ line ++ ","
  count = count + 1
}}

fn main(): Unit [io] = {{
  val code = Process.lines("{prog}", [{args}], (line) => collect(line))
  println("code=" ++ intToText(code))
  println("count=" ++ intToText(count))
  println("seen=" ++ seen)
}}
"####));
    assert_eq!(out, "code=0\ncount=3\nseen=alpha,beta,gamma,\n");
}

#[test]
fn process_lines_streams_output_larger_than_a_pipe_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for i in 0..5000 { body.push_str(&format!("row {i:05} {}\n", "x".repeat(40))); }
    fs::write(dir.path().join("big.txt"), &body).unwrap();
    let (prog, args) = print_file_command("big.txt");
    let out = run_program_in(dir.path(), &format!(r####"module P
var count = 0

fn tally(line: Text): Unit = {{
  count = count + 1
}}

fn main(): Unit [io] = {{
  val code = Process.lines("{prog}", [{args}], (line) => tally(line))
  println(intToText(code) ++ " " ++ intToText(count))
}}
"####));
    assert_eq!(out, "0 5000\n");
}

#[test]
fn process_lines_reports_a_missing_program_without_calling_the_handler() {
    let out = run_program(r####"module P
var count = 0

fn tally(line: Text): Unit = {
  count = count + 1
}

fn main(): Unit [io] = {
  val code = Process.lines("certo-no-such-program-xyz", [], (line) => tally(line))
  println(intToText(count))
  println(boolToText(code != 0))
}
"####);
    assert_eq!(out, "0\ntrue\n");
}
