use std::{fs, process::Command};

fn run_certo(source: &str) -> std::process::Output {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_test.cto");
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
fn failed_worker_restarts_and_becomes_healthy() {
    let stdout = assert_success(&run_certo(
        r#"module HostRestartTest

fn attemptsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("attempts")
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn work(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, attemptsKey()) {
        Some(attempts) => match Channel.tryReceive(attempts) {
            Some(_) => Err("transient")
            None => {
                HostContext.ready(c)
                while HostContext.sleep(c, Duration.seconds(5)) {}
                Ok(())
            }
        }
        None => Err("attempt service missing")
    }
}

fn main(): Unit [io] = {
    val attempts = Channel.new(capacity: 1)
    Channel.send(attempts, 1)
    val host = Host.new()
        .provide(attemptsKey(), attempts)
        .add(Host.plugin("p", start, stop)
            .worker("w", work)
            .restart(RestartPolicy.onFailure(
                2, Duration.milliseconds(1), Duration.milliseconds(5))))
    match Host.start(host) {
        Ok(_) => {
            println(Host.health(host))
            match Host.workerRestarts(host, "w") {
                Some(count) => println(intToText(count))
                None => println("missing")
            }
            val stopped = Host.stop(host)
        }
        Err(error) => println(f"unexpected: {error}")
    }
}
"#,
    ));
    assert!(stdout.contains("Healthy"), "{stdout}");
    assert!(stdout.lines().any(|line| line.trim() == "1"), "{stdout}");
}

#[test]
fn retry_exhaustion_fails_startup() {
    let stdout = assert_success(&run_certo(
        r#"module HostExhaustionTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn fail(c: HostContext): Result<Unit, Text> = Err("still broken")

fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("p", start, stop)
        .worker("w", fail)
        .restart(RestartPolicy.onFailure(
            2, Duration.milliseconds(1), Duration.milliseconds(2))))
    match Host.start(host) {
        Ok(_) => println("unexpected success")
        Err(error) => {
            println(error)
            println(Host.health(host))
        }
    }
}
"#,
    ));
    assert!(stdout.contains("still broken"), "{stdout}");
    assert!(stdout.contains("Failed"), "{stdout}");
    assert!(!stdout.contains("unexpected success"), "{stdout}");
}

#[test]
fn readiness_timeout_cancels_the_worker() {
    let stdout = assert_success(&run_certo(
        r#"module HostReadinessTimeoutTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn wait(c: HostContext): Result<Unit, Text> [io] = {
    val completed = HostContext.sleep(c, Duration.seconds(5))
    Ok(())
}

fn main(): Unit [io] = {
    val host = Host.new()
        .readinessTimeout(Duration.milliseconds(5))
        .add(Host.plugin("p", start, stop).worker("w", wait))
    match Host.start(host) {
        Ok(_) => println("unexpected success")
        Err(error) => println(error)
    }
}
"#,
    ));
    assert!(stdout.contains("readiness timed out"), "{stdout}");
}

#[test]
fn shutdown_aggregates_quiesce_and_stop_errors() {
    let stdout = assert_success(&run_certo(
        r#"module HostShutdownErrorsTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn quiesce(c: HostContext): Result<Unit, Text> = Err("quiesce broke")
fn stop(c: HostContext): Result<Unit, Text> = Err("stop broke")
fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    while HostContext.sleep(c, Duration.seconds(5)) {}
    Ok(())
}

fn main(): Unit [io] = {
    val plugin = HostPlugin.worker(
        HostPlugin.quiesce(Host.plugin("p", start, stop), quiesce),
        "w",
        work)
    val host = Host.new().add(plugin)
    match Host.start(host) {
        Ok(_) => match Host.stop(host) {
            Ok(_) => println("unexpected success")
            Err(error) => println(error)
        }
        Err(error) => println(f"unexpected start: {error}")
    }
    match Host.stop(host) {
        Ok(_) => println("unexpected second stop success")
        Err(error) => println(error)
    }
}
"#,
    ));
    assert_eq!(stdout.matches("quiesce broke").count(), 2, "{stdout}");
    assert_eq!(stdout.matches("stop broke").count(), 2, "{stdout}");
}

#[test]
fn duplicate_plugin_names_are_rejected_before_startup() {
    let output = run_certo(
        r#"module HostDuplicatePluginTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit = {
    val host = Host.new()
        .add(Host.plugin("same", start, stop))
        .add(Host.plugin("same", start, stop))
}
"#,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate plugin name"),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn duplicate_worker_names_are_rejected_before_startup() {
    let output = run_certo(
        r#"module HostDuplicateWorkerTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit = {
    val plugin = Host.plugin("p", start, stop)
        .worker("same", work)
        .worker("same", work)
    val host = Host.new().add(plugin)
}
"#,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate worker name"),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn contradictory_restart_delays_are_rejected() {
    let output = run_certo(
        r#"module HostRestartValidationTest
fn main(): Unit = {
    val policy = RestartPolicy.onFailure(
        2,
        Duration.seconds(2),
        Duration.seconds(1))
}
"#,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "restart maxDelay cannot be less than initialDelay"
        ),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn lifecycle_transitions_are_one_way_and_stop_is_idempotent() {
    let stdout = assert_success(&run_certo(
        r#"module HostLifecycleStateTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("p", start, stop))
    match Host.start(host) {
        Ok(_) => println("started")
        Err(error) => println(f"unexpected start: {error}")
    }
    match Host.start(host) {
        Ok(_) => println("unexpected second start")
        Err(error) => println(error)
    }
    match Host.stop(host) {
        Ok(_) => println("stopped")
        Err(error) => println(f"unexpected stop: {error}")
    }
    match Host.stop(host) {
        Ok(_) => println("stopped again")
        Err(error) => println(f"unexpected second stop: {error}")
    }
    match Host.start(host) {
        Ok(_) => println("unexpected restart")
        Err(error) => println(error)
    }
}
"#,
    ));

    assert!(stdout.contains("host has already started"), "{stdout}");
    assert!(stdout.contains("stopped again"), "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}
