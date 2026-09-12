use std::{fs, process::Command};

#[test]
fn logical_operators_skip_rhs_across_value_and_control_flow_positions() {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("short_circuit.cto");
    fs::write(
        &source_path,
        r#"module ShortCircuitTest

fn called(label: Text, value: Bool): Bool [io] = {
    println(label)
    value
}

fn main(): Unit [io] = {
    val a = false and called("bad-and-value", true)
    val b = true or called("bad-or-value", false)
    val c = true and called("good-and", true)
    val d = false or called("good-or", true)

    if false and called("bad-and-if", true) then println("bad-if-body") else ()
    if true or called("bad-or-if", false) then () else println("bad-else-body")

    var keepGoing = false
    while keepGoing and called("bad-and-while", true) {
        keepGoing = false
    }

    if a or b or c or d then println("complete") else ()
}
"#,
    )
    .expect("write Certo source");

    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .output()
        .expect("run Certo short-circuit program");
    assert!(
        output.status.success(),
        "certo failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.lines().any(|line| line == "good-and"), "{stdout}");
    assert!(stdout.lines().any(|line| line == "good-or"), "{stdout}");
    assert!(stdout.lines().any(|line| line == "complete"), "{stdout}");
    assert!(!stdout.contains("bad-"), "{stdout}");
}
