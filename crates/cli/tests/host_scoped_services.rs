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

#[test]
fn scoped_disposal_timeout_retains_reachable_services_and_continues_cleanup() {
    let stdout = assert_success(&run_certo(r#"module HostScopedDisposalTimeoutTest

type Resource = { value: Int }
fn eventsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("events")
fn hostKey(): ServiceKey<Resource> = Host.serviceKey("host-resource")
fn aKey(): ServiceKey<Resource> = Host.serviceKey("a-resource")
fn bKey(): ServiceKey<Resource> = Host.serviceKey("b-resource")

fn emit(c: HostContext, value: Int): Unit [io] = {
    match HostContext.service(c, eventsKey()) {
        Some(events) => Channel.send(events, value)
        None => ()
    }
}
fn create(c: HostContext): Result<Resource, Text> = Ok(Resource { value: 7 })
fn disposeHost(c: HostContext): Result<Unit, Text> [io] = {
    emit(c, 9)
    Ok(())
}
fn disposeA(c: HostContext): Result<Unit, Text> [io] = {
    emit(c, 2)
    Ok(())
}
fn disposeB(c: HostContext): Result<Unit, Text> [io] = {
    emit(c, 1)
    sleep(50)
    match HostContext.service(c, bKey()) {
        Some(_) => emit(c, 3)
        None => emit(c, 30)
    }
    match HostContext.service(c, hostKey()) {
        Some(_) => emit(c, 4)
        None => emit(c, 40)
    }
    Ok(())
}
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val events = Channel.new(capacity: 8)
    val a = Host.plugin("a", start, stop).provideFactory(aKey(), create, disposeA)
    val b = Host.plugin("b", start, stop).provideFactory(bKey(), create, disposeB)
    val host = Host.new()
        .disposalTimeout(Duration.milliseconds(5))
        .provide(eventsKey(), events)
        .provideFactory(hostKey(), create, disposeHost)
        .add(a)
        .add(b)
    val started = Host.start(host)
    match Host.stopTyped(host) {
        Ok(_) => println("unexpected success")
        Err(error) => {
            println(HostFailureKind.name(HostLifecycleError.kind(error)))
            println(HostLifecycleError.phase(error))
            println(HostLifecycleError.subject(error))
            println(boolToText(HostLifecycleError.isTimeout(error)))
        }
    }
    sleep(75)
    for i in 0..3 {
        match Channel.tryReceive(events) {
            Some(value) => println(intToText(value))
            None => println("missing event")
        }
    }
}
"#));

    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["Timeout", "dispose", "b/b-resource", "true", "1", "2", "3", "4"],
        "{stdout}"
    );
    assert!(!stdout.contains("9"), "host service disposer must be retained: {stdout}");
    assert!(!stdout.contains("30"), "timed-out scoped service became unavailable: {stdout}");
    assert!(!stdout.contains("40"), "host dependency became unavailable: {stdout}");
}
