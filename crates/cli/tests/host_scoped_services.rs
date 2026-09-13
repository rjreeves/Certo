use std::{fs, process::Command};

fn run_certo(source: &str) -> std::process::Output {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_scoped_service_test.cto");
    fs::write(&source_path, source).expect("write Certo source");
    Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .output()
        .expect("run certo")
}

fn assert_success(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "certo failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn plugin_scoped_services_are_isolated_ordered_and_disposed_with_their_plugin() {
    let stdout = assert_success(&run_certo(include_str!("fixtures/host_plugin_scope.cto")));
    assert_eq!(stdout.lines().collect::<Vec<_>>(), [
        "a-base-create", "a-derived-create", "11", "b-hidden",
        "b-create", "20", "a-hidden", "b-worker-20", "a-worker-hidden",
        "b-stop", "b-dispose",
        "a-stop", "a-derived-dispose", "a-base-dispose",
    ]);
    assert!(!stdout.contains("leaked"), "{stdout}");
    assert!(!stdout.contains("missing"), "{stdout}");
}

#[test]
fn cross_plugin_scoped_dependency_is_rejected_before_callbacks() {
    let stdout = assert_success(&run_certo(include_str!(
        "fixtures/host_plugin_scope_validation.cto"
    )));
    assert!(stdout.contains("missing scoped dependency"), "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}

#[test]
fn scoped_dependency_cycle_is_rejected_before_callbacks() {
    let stdout = assert_success(&run_certo(include_str!(
        "fixtures/host_plugin_scope_cycle.cto"
    )));
    assert!(stdout.contains("dependencies form a cycle"), "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}

#[test]
fn scoped_factory_failure_rolls_back_current_and_started_plugin_scopes() {
    let stdout = assert_success(&run_certo(include_str!(
        "fixtures/host_plugin_scope_rollback.cto"
    )));
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(&lines[..7], [
        "a-create", "a-start", "b-create", "b-fail",
        "b-dispose", "a-stop", "a-dispose",
    ]);
    assert!(lines[7].contains("failed to construct scoped service"), "{stdout}");
}
