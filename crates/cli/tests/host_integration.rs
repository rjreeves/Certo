use std::{env, fs, process::Command};

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

const HOST_STRESS_SOURCE: &str = r#"module HostStressTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn quiesce(c: HostContext): Result<Unit, Text> [io] = {
    sleep(2)
    println("quiesced")
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> [io] = {
    println("stopped")
    Ok(())
}
fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    while HostContext.sleep(c, Duration.milliseconds(1)) {}
    Ok(())
}

fn main(): Unit [io, async] = {
    var iteration = 0
    while iteration < 50 {
        val plugin = HostPlugin.worker(
            HostPlugin.quiesce(Host.plugin("plugin", start, stop), quiesce),
            "worker",
            work)
        val host = Host.new().add(plugin)
        match Host.start(host) {
            Ok(_) => {
                val stopped = await parallel {
                    Host.stop(host),
                    Host.stop(host),
                    Host.stop(host),
                    Host.stop(host),
                }
            }
            Err(error) => println(f"start failed: {error}")
        }
        iteration = iteration + 1
    }
    println("stress complete")
}
"#;

const HOST_SHUTDOWN_RACE_SOURCE: &str = r#"module HostShutdownRaceTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn failureSignalKey(): ServiceKey<Channel<Int>> = Host.serviceKey("failure-signal")

fn delayedReady(c: HostContext): Result<Unit, Text> [io] = {
    val completed = HostContext.sleep(c, Duration.milliseconds(1))
    HostContext.ready(c)
    while HostContext.sleep(c, Duration.milliseconds(1)) {}
    Ok(())
}

fn failAfterReady(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    match HostContext.service(c, failureSignalKey()) {
        Some(signal) => {
            val fired = Channel.receive(signal)
            Err("intentional race failure")
        }
        None => Err("failure signal missing")
    }
}

fn triggerFailure(signal: Channel<Int>): Result<Unit, Text> = {
    Channel.send(signal, 1)
    Ok(())
}

fn main(): Unit [io, async] = {
    var iteration = 0
    while iteration < 50 {
        val readinessHost = Host.new()
            .readinessTimeout(Duration.seconds(1))
            .add(Host.plugin("readiness", start, stop)
                .worker("delayed-ready", delayedReady))
        val startupRace = await parallel {
            Host.start(readinessHost),
            Host.stop(readinessHost),
        }
        val readinessCleanup = Host.stop(readinessHost)

        val failureSignal = Channel.new(capacity: 1)
        val failureHost = Host.new()
            .readinessTimeout(Duration.seconds(1))
            .provide(failureSignalKey(), failureSignal)
            .add(Host.plugin("failure", start, stop)
                .worker("failing", failAfterReady)
                .restart(RestartPolicy.onFailure(
                    100, Duration.milliseconds(5), Duration.milliseconds(5))))
        match Host.start(failureHost) {
            Ok(_) => {
                val shutdownRace = await parallel {
                    triggerFailure(failureSignal),
                    Host.stop(failureHost),
                    Host.stop(failureHost),
                    Host.stop(failureHost),
                    Host.stop(failureHost),
                }
            }
            Err(_) => {}
        }
        val failureCleanup = Host.stop(failureHost)
        iteration = iteration + 1
    }
    println("shutdown race complete")
}
"#;

const HOST_RANDOMIZED_STRESS_SOURCE: &str = r#"module HostRandomizedStressTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn readyDelayKey(): ServiceKey<Channel<Int>> = Host.serviceKey("ready-delay")
fn failureSignalKey(): ServiceKey<Channel<Int>> = Host.serviceKey("failure-signal")

fn delayedReady(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, readyDelayKey()) {
        Some(delays) => match Channel.receive(delays) {
            Some(delay) => {
                val completed = HostContext.sleep(c, Duration.milliseconds(delay))
                HostContext.ready(c)
                while HostContext.sleep(c, Duration.milliseconds(1)) {}
                Ok(())
            }
            None => Err("ready delay channel closed")
        }
        None => Err("ready delay service missing")
    }
}

fn failWhenSignalled(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    var failed = false
    while !failed and HostContext.sleep(c, Duration.milliseconds(1)) {
        match HostContext.service(c, failureSignalKey()) {
            Some(signal) => match Channel.tryReceive(signal) {
                Some(_) => { failed = true }
                None => {}
            }
            None => {}
        }
    }
    if failed then Err("randomized race failure") else Ok(())
}

fn startAfter(host: Host, delay: Int): Result<Unit, Text> [io] = {
    sleep(delay)
    Host.start(host)
}

fn stopAfter(host: Host, delay: Int): Result<Unit, Text> [io] = {
    sleep(delay)
    Host.stop(host)
}

fn failAfter(signal: Channel<Int>, delay: Int): Result<Unit, Text> [io] = {
    sleep(delay)
    Channel.send(signal, 1)
    Ok(())
}

fn main(): Unit [io, async] = {
    var state = __SEED__
    var iteration = 0
    while iteration < __ITERATIONS__ {
        state = (state * 48271) % 2147483647
        val readyDelay = state % 5
        state = (state * 48271) % 2147483647
        val startDelay = state % 5
        state = (state * 48271) % 2147483647
        val startupStopDelay = state % 5

        val readyDelays = Channel.new(capacity: 1)
        Channel.send(readyDelays, readyDelay)
        val readinessHost = Host.new()
            .readinessTimeout(Duration.seconds(1))
            .provide(readyDelayKey(), readyDelays)
            .add(Host.plugin("readiness", start, stop)
                .worker("delayed-ready", delayedReady))
        val startupRace = await parallel {
            startAfter(readinessHost, startDelay),
            stopAfter(readinessHost, startupStopDelay),
        }
        val readinessCleanup = Host.stop(readinessHost)

        state = (state * 48271) % 2147483647
        val failureDelay = state % 5
        state = (state * 48271) % 2147483647
        val stopDelayA = state % 5
        state = (state * 48271) % 2147483647
        val stopDelayB = state % 5
        state = (state * 48271) % 2147483647
        val stopDelayC = state % 5

        val failureSignal = Channel.new(capacity: 1)
        val failureHost = Host.new()
            .readinessTimeout(Duration.seconds(1))
            .provide(failureSignalKey(), failureSignal)
            .add(Host.plugin("failure", start, stop)
                .worker("failing", failWhenSignalled)
                .restart(RestartPolicy.onFailure(
                    100, Duration.milliseconds(1), Duration.milliseconds(5))))
        match Host.start(failureHost) {
            Ok(_) => {
                val failureRace = await parallel {
                    failAfter(failureSignal, failureDelay),
                    stopAfter(failureHost, stopDelayA),
                    stopAfter(failureHost, stopDelayB),
                    stopAfter(failureHost, stopDelayC),
                }
            }
            Err(_) => {}
        }
        val settled = Host.waitUntilReady(failureHost, Duration.seconds(1))
        val failureCleanup = Host.stop(failureHost)
        iteration = iteration + 1
    }
    println("randomized lifecycle stress complete")
}
"#;

#[test]
fn repeated_host_lifecycles_are_stable() {
    let stdout = assert_success(&run_certo(HOST_STRESS_SOURCE));
    assert!(stdout.contains("stress complete"), "{stdout}");
    assert!(!stdout.contains("failed:"), "{stdout}");
    assert_eq!(
        stdout.lines().filter(|line| *line == "quiesced").count(),
        50,
        "{stdout}"
    );
    assert_eq!(
        stdout.lines().filter(|line| *line == "stopped").count(),
        50,
        "{stdout}"
    );
}

#[test]
fn shutdown_races_are_stable() {
    let stdout = assert_success(&run_certo(HOST_SHUTDOWN_RACE_SOURCE));
    assert!(stdout.contains("shutdown race complete"), "{stdout}");
}

#[cfg(not(windows))]
#[test]
fn sanitizer_stress_has_no_native_memory_errors() {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_sanitizer_stress.cto");
    fs::write(&source_path, HOST_STRESS_SOURCE).expect("write Certo source");
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .arg("--sanitize-address")
        .env("CC", "clang")
        .env(
            "ASAN_OPTIONS",
            "detect_leaks=0:halt_on_error=1:abort_on_error=1",
        )
        .output()
        .expect("run sanitizer-backed Certo host stress test");
    let stdout = assert_success(&output);
    assert!(stdout.contains("stress complete"), "{stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn thread_sanitizer_stress_has_no_data_races() {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_thread_sanitizer_stress.cto");
    fs::write(&source_path, HOST_STRESS_SOURCE).expect("write Certo source");
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .arg("--sanitize-thread")
        .env("CC", "clang")
        .env("TSAN_OPTIONS", "halt_on_error=1:abort_on_error=1")
        .output()
        .expect("run ThreadSanitizer-backed Certo host stress test");
    let stdout = assert_success(&output);
    assert!(stdout.contains("stress complete"), "{stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn thread_sanitizer_shutdown_race_has_no_data_races() {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_thread_sanitizer_shutdown_race.cto");
    fs::write(&source_path, HOST_SHUTDOWN_RACE_SOURCE).expect("write Certo source");
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .arg("--sanitize-thread")
        .env("CC", "clang")
        .env("TSAN_OPTIONS", "halt_on_error=1:abort_on_error=1")
        .output()
        .expect("run ThreadSanitizer-backed host shutdown race test");
    let stdout = assert_success(&output);
    assert!(stdout.contains("shutdown race complete"), "{stdout}");
}

#[test]
#[ignore = "scheduled sanitizer stress"]
fn randomized_lifecycle_stress_is_reproducible() {
    let raw_seed = env::var("CERTO_HOST_STRESS_SEED").unwrap_or_else(|_| "1".to_owned());
    let parsed_seed = raw_seed
        .parse::<i64>()
        .expect("CERTO_HOST_STRESS_SEED must be an integer");
    let normalized_seed = parsed_seed.rem_euclid(2_147_483_647);
    let seed = if normalized_seed == 0 {
        1
    } else {
        normalized_seed
    };
    let iterations = env::var("CERTO_HOST_STRESS_ITERATIONS")
        .unwrap_or_else(|_| "50".to_owned())
        .parse::<i64>()
        .expect("CERTO_HOST_STRESS_ITERATIONS must be an integer")
        .clamp(1, 10_000);
    eprintln!("randomized host lifecycle stress: seed={seed} iterations={iterations}");

    let source = HOST_RANDOMIZED_STRESS_SOURCE
        .replace("__SEED__", &seed.to_string())
        .replace("__ITERATIONS__", &iterations.to_string());
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_randomized_lifecycle_stress.cto");
    fs::write(&source_path, source).expect("write Certo source");

    let mut command = Command::new(env!("CARGO_BIN_EXE_certo"));
    command.arg("run").arg(&source_path);
    match env::var("CERTO_HOST_STRESS_SANITIZER").as_deref() {
        Ok("address") => {
            command.arg("--sanitize-address").env("CC", "clang").env(
                "ASAN_OPTIONS",
                "detect_leaks=0:halt_on_error=1:abort_on_error=1",
            );
        }
        Ok("thread") => {
            command
                .arg("--sanitize-thread")
                .env("CC", "clang")
                .env("TSAN_OPTIONS", "halt_on_error=1:abort_on_error=1");
        }
        Ok(other) => panic!("unknown CERTO_HOST_STRESS_SANITIZER: {other}"),
        Err(_) => {}
    }
    let output = command
        .output()
        .expect("run randomized host lifecycle stress test");
    let stdout = assert_success(&output);
    assert!(
        stdout.contains("randomized lifecycle stress complete"),
        "{stdout}"
    );
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
        String::from_utf8_lossy(&output.stderr)
            .contains("restart maxDelay cannot be less than initialDelay"),
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
