use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

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
fn operational_probe_adapters_map_lifecycle_states_and_metrics() {
    let output = run_certo(r#"module HostOperationalProbeTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn failStart(c: HostContext): Result<Unit, Text> = Err("startup failed")

fn show(host: Host): Unit [io] = {
    val status = Host.operationStatus(host)
    println(HostOperationalCondition.name(HostOperationalStatus.condition(status)))
    println(HostState.name(HostOperationalStatus.state(status)))
    println(boolToText(HostOperationalStatus.isLive(status)))
    println(boolToText(HostOperationalStatus.isReady(status)))
    val live = HostHttp.liveness(host)
    val ready = HostHttp.readiness(host)
    println(intToText(HttpResponse.status(live)))
    println(HttpResponse.body(live))
    println(intToText(HttpResponse.status(ready)))
    println(HttpResponse.body(ready))
}

fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("probe", start, stop))
    val requests = Host.counterMetric(host, "probe.requests", "Probe requests", "requests", [], 1)
    show(host)
    val started = Host.start(host)
    show(host)
    HostMetric.counterAdd(requests, [], 2)
    val metrics = HostHttp.metrics(host)
    println(intToText(HttpResponse.status(metrics)))
    println(HttpResponse.contentType(metrics))
    println(HttpResponse.body(metrics))
    val stopped = Host.stop(host)
    show(host)

    val failed = Host.new().add(Host.plugin("failure", failStart, stop))
    val failedStartResult = Host.start(failed)
    show(failed)
    match HostOperationalStatus.failureKind(Host.operationStatus(failed)) {
        Some(kind) => println(HostFailureKind.name(kind))
        None => println("missing")
    }
}
"#);
    let stdout = assert_success(&output).replace("\r\n", "\n");
    assert!(stdout.contains(concat!(
        "starting\nNew\ntrue\nfalse\n",
        "200\n{\"condition\":\"starting\",\"state\":\"New\",\"live\":true,\"ready\":false}\n",
        "503\n{\"condition\":\"starting\",\"state\":\"New\",\"live\":true,\"ready\":false}\n",
    )), "{stdout}");
    assert!(stdout.contains(concat!(
        "ready\nHealthy\ntrue\ntrue\n",
        "200\n{\"condition\":\"ready\",\"state\":\"Healthy\",\"live\":true,\"ready\":true}\n",
        "200\n{\"condition\":\"ready\",\"state\":\"Healthy\",\"live\":true,\"ready\":true}\n",
    )), "{stdout}");
    assert!(stdout.contains("200\ntext/plain; version=0.0.4; charset=utf-8\n"), "{stdout}");
    assert!(stdout.contains("probe_requests_total 2\n"), "{stdout}");
    assert!(stdout.contains(concat!(
        "stopped\nStopped\nfalse\nfalse\n",
        "503\n{\"condition\":\"stopped\",\"state\":\"Stopped\",\"live\":false,\"ready\":false}\n",
        "503\n{\"condition\":\"stopped\",\"state\":\"Stopped\",\"live\":false,\"ready\":false}\n",
    )), "{stdout}");
    assert!(stdout.contains(
        "{\"condition\":\"failed\",\"state\":\"Failed\",\"live\":false,\"ready\":false,\"failure_kind\":\"StartupFailure\"}"
    ), "{stdout}");
    assert!(stdout.ends_with("StartupFailure\n"), "{stdout}");
}

#[test]
fn operational_condition_distinguishes_starting_degraded_and_stopping() {
    let output = run_certo(r#"module HostOperationalTransitionTest

fn signalKey(): ServiceKey<Channel<Int>> = Host.serviceKey("failure-signal")
fn attemptsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("attempts")
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn slowStart(c: HostContext): Result<Unit, Text> [io] = { sleep(100) Ok(()) }
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn slowStop(c: HostContext): Result<Unit, Text> [io] = { sleep(100) Ok(()) }
fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    match HostContext.service(c, attemptsKey()) {
        Some(attempts) => match Channel.tryReceive(attempts) {
            Some(_) => match HostContext.service(c, signalKey()) {
                Some(signal) => match Channel.receive(signal) {
                    Some(_) => Err("restart")
                    None => Ok(())
                }
                None => Err("signal missing")
            }
            None => {
                while HostContext.sleep(c, Duration.seconds(5)) {}
                Ok(())
            }
        }
        None => Err("attempts missing")
    }
}
fn printCondition(label: Text, host: Host): Unit [io] = {
    val status = Host.operationStatus(host)
    println(f"{label}:{HostOperationalCondition.name(HostOperationalStatus.condition(status))}")
}
fn startHost(host: Host): Unit [io] = { val result = Host.start(host) }
fn stopHost(host: Host): Unit [io] = { val result = Host.stop(host) }
fn observeStarting(host: Host): Unit [io] = { sleep(20) printCondition("startup", host) }
fn observeStopping(host: Host): Unit [io] = { sleep(20) printCondition("shutdown", host) }

fn main(): Unit [io, async] = {
    val startingHost = Host.new().add(Host.plugin("slow", slowStart, stop))
    val startup = await parallel { startHost(startingHost), observeStarting(startingHost) }
    val startingCleanup = Host.stop(startingHost)

    val stoppingHost = Host.new().add(Host.plugin("slow", start, slowStop))
    val stoppingStarted = Host.start(stoppingHost)
    val shutdown = await parallel { stopHost(stoppingHost), observeStopping(stoppingHost) }

    val signal = Channel.new(capacity: 1)
    val attempts = Channel.new(capacity: 1)
    Channel.send(attempts, 1)
    val degradedHost = Host.new()
        .provide(signalKey(), signal)
        .provide(attemptsKey(), attempts)
        .add(Host.plugin("restart", start, stop)
            .worker("worker", work)
            .restart(RestartPolicy.onFailure(
                2, Duration.milliseconds(200), Duration.milliseconds(200))))
    val degradedStarted = Host.start(degradedHost)
    Channel.send(signal, 1)
    sleep(20)
    printCondition("restart", degradedHost)
    val degradedCleanup = Host.stop(degradedHost)
}
"#);
    let stdout = assert_success(&output);
    assert!(stdout.contains("startup:starting"), "{stdout}");
    assert!(stdout.contains("shutdown:stopping"), "{stdout}");
    assert!(stdout.contains("restart:degraded"), "{stdout}");
}

#[test]
fn typed_host_events_preserve_schema_fields_context_and_sequence() {
    let output = run_certo(r#"module HostTypedEventTest

fn start(c: HostContext): Result<Unit, Text> [io] = {
    val event = HostLogEvent.create(HostLogSeverity.info(), "job.started", "starting", [
        HostLogField.text("queue", "critical"),
        HostLogField.int("attempt", 2),
        HostLogField.float("ratio", 0.5),
        HostLogField.bool("retrying", true)
    ])
    HostContext.logEvent(c, event)
    println(HostLogEvent.schema(event))
    println(intToText(HostLogEvent.sequence(event)))
    println(HostLogSeverity.name(HostLogEvent.severity(event)))
    HostContext.log(c, "wArN", "job.waiting", "waiting")
    Ok(())
}

fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("typed", start, stop))
    val started = Host.start(host)
    val stopped = Host.stop(host)
}
"#);
    let stdout = assert_success(&output);
    assert_eq!(stdout.lines().collect::<Vec<_>>(), ["certo.host.event/v1", "0", "Info"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines = stderr.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2, "{stderr}");
    assert!(lines[0].contains("\"schema\":\"certo.host.event/v1\""), "{stderr}");
    assert!(lines[0].contains("\"sequence\":1"), "{stderr}");
    assert!(lines[0].contains("\"timestamp_unix_ms\":"), "{stderr}");
    assert!(lines[0].contains("\"severity\":\"Info\""), "{stderr}");
    assert!(lines[0].contains("\"plugin\":\"typed\""), "{stderr}");
    assert!(lines[0].contains("\"worker\":null"), "{stderr}");
    assert!(lines[0].contains("\"correlation_id\":null"), "{stderr}");
    assert!(lines[0].contains("\"fields\":{\"queue\":\"critical\",\"attempt\":2,\"ratio\":0.5,\"retrying\":true}"), "{stderr}");
    assert!(lines[1].contains("\"sequence\":2"), "{stderr}");
    assert!(lines[1].contains("\"severity\":\"Warn\""), "{stderr}");
}

#[test]
fn correlation_context_is_inherited_derived_isolated_and_explicit_for_metrics() {
    let output = run_certo(r#"module HostCorrelationContextTest

fn metricKey(): ServiceKey<HostMetric> = Host.serviceKey("correlation-metric")

fn start(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.log(c, "info", "correlation.root", "root")
    val derived = HostContext.withCorrelation(c, "operation-7")
    HostContext.log(derived, "info", "correlation.derived", "derived")
    HostContext.log(c, "info", "correlation.parent", "parent")
    match HostContext.service(c, metricKey()) {
        Some(metric) => HostMetric.counterAdd(
            metric, [HostContext.correlationId(derived) ?? "missing"], 1)
        None => ()
    }
    Ok(())
}

fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    HostContext.log(c, "info", "correlation.worker", "worker")
    Ok(())
}

fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val host = Host.new()
    val metric = Host.counterMetric(
        host, "operations", "Correlated operations", "operations",
        ["correlation_id"], 4)
    host.provide(metricKey(), metric)
        .add(Host.plugin("correlated", start, stop).worker("worker", work))
        .correlation("request-42")
    val started = Host.start(host)
    val stopped = Host.stop(host)
    println(Host.metricSnapshot(host))
}
"#);
    let stdout = assert_success(&output);
    assert!(stdout.contains("\"labels\":[\"operation-7\"]"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines = stderr.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 4, "{stderr}");
    assert!(lines[0].contains("\"correlation_id\":\"request-42\""), "{stderr}");
    assert!(lines[1].contains("\"correlation_id\":\"operation-7\""), "{stderr}");
    assert!(lines[2].contains("\"correlation_id\":\"request-42\""), "{stderr}");
    assert!(lines[3].contains("\"correlation_id\":\"request-42\""), "{stderr}");
    assert!(lines[3].contains("\"plugin\":\"correlated\""), "{stderr}");
    assert!(lines[3].contains("\"worker\":\"worker\""), "{stderr}");
}

#[test]
fn exposed_secret_configuration_is_redacted_from_events_and_failures() {
    const CANARY: &str = "h4-secret-canary-91f6";
    let output = run_certo(r#"module HostSecretObservabilityTest

type Secret<T> = | Hidden
fn parseSecret(value: Text): Result<Secret<Text>, Text> = Ok(Hidden)
fn secretKey(): ConfigKey<Secret<Text>> = Host.configKey("secret.token", parseSecret)

fn start(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.log(c, "info", "secret.event", "message h4-secret-canary-91f6")
    HostContext.logEvent(c, HostLogEvent.create(
        HostLogSeverity.info(), "secret.field", "field",
        [HostLogField.text("value", "h4-secret-canary-91f6")]))
    Err("startup h4-secret-canary-91f6")
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val host = Host.new()
        .configure("secret.token", "h4-secret-canary-91f6")
        .requireConfig(secretKey())
        .add(Host.plugin("secret", start, stop))
    match Host.startTyped(host) {
        Ok(_) => println("unexpected")
        Err(error) => {
            println(HostLifecycleError.message(error))
            match HostStatusSnapshot.lastFailure(Host.status(host)) {
                Some(last) => println(HostLifecycleError.message(last))
                None => println("missing")
            }
            println(Host.metrics(host))
            println(Host.metricSnapshot(host))
            println(Host.metricsPrometheus(host))
        }
    }
}
"#);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(CANARY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(CANARY));
    let stdout = assert_success(&output);
    assert!(stdout.contains("[REDACTED]"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("message [REDACTED]"), "{stderr}");
    assert!(stderr.contains("\"value\":\"[REDACTED]\""), "{stderr}");
}

#[test]
fn exposed_secret_configuration_is_rejected_from_metric_labels() {
    const CANARY: &str = "h4-metric-secret-canary-44a2";
    let output = run_certo(r#"module HostSecretMetricTest

type Secret<T> = | Hidden
fn parseSecret(value: Text): Result<Secret<Text>, Text> = Ok(Hidden)
fn secretKey(): ConfigKey<Secret<Text>> = Host.configKey("secret.token", parseSecret)
fn metricKey(): ServiceKey<HostMetric> = Host.serviceKey("metric")

fn start(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, metricKey()) {
        Some(metric) => HostMetric.counterAdd(metric, ["h4-metric-secret-canary-44a2"], 1)
        None => ()
    }
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val host = Host.new().configure("secret.token", "h4-metric-secret-canary-44a2")
    val metric = Host.counterMetric(host, "requests", "Requests", "requests", ["key"], 2)
    host.requireConfig(secretKey()).provide(metricKey(), metric)
        .add(Host.plugin("secret", start, stop))
    val started = Host.start(host)
}
"#);
    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains(CANARY), "{stdout}");
    assert!(!stderr.contains(CANARY), "{stderr}");
    assert!(stderr.contains("secret configuration cannot be used as a metric label value"), "{stderr}");
}

#[test]
fn pluggable_log_sink_drains_in_order_and_finalizes() {
    let output = run_certo(r#"module HostLogSinkTest

fn writeEvent(event: HostLogEvent): Result<Unit, Text> [io] = {
    println(f"event:{HostLogEvent.sequence(event)}:{HostLogEvent.event(event)}")
    Ok(())
}

fn flushSink(): Result<Unit, Text> [io] = {
    println("flush")
    Ok(())
}
fn disposeSink(): Result<Unit, Text> [io] = {
    println("dispose")
    Ok(())
}
fn start(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.log(c, "info", "test.one", "one")
    HostContext.log(c, "warn", "test.two", "two")
    HostContext.log(c, "error", "test.three", "three")
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit [io] = {
    val host = Host.new()
        .disableStderrLog()
        .telemetryTimeout(Duration.seconds(2))
        .logSink("capture", 8, HostLogOverflowPolicy.dropNewest(),
            HostLogFailurePolicy.ignore(), writeEvent, flushSink, disposeSink)
        .add(Host.plugin("producer", start, stop))
    val started = Host.start(host)
    val stopped = Host.stop(host)
}
"#);
    let stdout = assert_success(&output);
    assert_eq!(stdout.lines().collect::<Vec<_>>(), [
        "event:1:test.one", "event:2:test.two", "event:3:test.three",
        "flush", "dispose",
    ]);
    assert!(String::from_utf8_lossy(&output.stderr).trim().is_empty());
}

#[test]
fn typed_metric_registry_is_labeled_bounded_and_deterministic() {
    let output = run_certo(r#"module HostTypedMetricsTest

fn main(): Unit [io] = {
    val host = Host.new()
    val requests = Host.counterMetric(host, "requests", "Accepted requests", "requests", ["route"], 2)
    val active = Host.gaugeMetric(host, "active", "Active jobs", "jobs", [], 1)
    val latency = Host.histogramMetric(host, "latency", "Request latency", "ms", ["route"], [10, 50, 100], 2)
    HostMetric.counterAdd(requests, ["/b"], 2)
    HostMetric.counterAdd(requests, ["/a"], 1)
    HostMetric.counterAdd(requests, ["/c"], 9)
    HostMetric.gaugeSet(active, [], -3)
    HostMetric.histogramObserve(latency, ["/a"], 7)
    HostMetric.histogramObserve(latency, ["/a"], 60)
    println(Host.metricSnapshot(host))
}
"#);
    let stdout = assert_success(&output);
    assert_eq!(stdout.trim(), concat!(
        "{\"instruments\":[",
        "{\"name\":\"requests\",\"kind\":\"Counter\",\"help\":\"Accepted requests\",\"unit\":\"requests\",\"labels\":[\"route\"],\"series\":[",
        "{\"labels\":[\"/a\"],\"value\":1},{\"labels\":[\"/b\"],\"value\":2}]},",
        "{\"name\":\"active\",\"kind\":\"Gauge\",\"help\":\"Active jobs\",\"unit\":\"jobs\",\"labels\":[],\"series\":[{\"labels\":[],\"value\":-3}]},",
        "{\"name\":\"latency\",\"kind\":\"Histogram\",\"help\":\"Request latency\",\"unit\":\"ms\",\"labels\":[\"route\"],\"series\":[",
        "{\"labels\":[\"/a\"],\"count\":2,\"sum\":67,\"buckets\":[{\"le\":10,\"count\":1},{\"le\":50,\"count\":1},{\"le\":100,\"count\":2}]}]}",
        "]}"
    ));
}

#[test]
fn prometheus_metrics_are_normalized_escaped_and_deterministic() {
    let output = run_certo(r#"module HostPrometheusMetricsTest

fn main(): Unit [io] = {
    val host = Host.new()
    val requests = Host.counterMetric(host, "http.requests", "Requests\\total\nnow", "requests", ["route-name"], 4)
    val latency = Host.histogramMetric(host, "http-latency", "Latency", "ms", ["route"], [10, 50], 4)
    HostMetric.counterAdd(requests, ["""GET "/x"
"""], 3)
    HostMetric.histogramObserve(latency, ["/x"], 12)
    println(Host.metricsPrometheus(host))
}
"#);
    let stdout = assert_success(&output).replace("\r\n", "\n");
    assert_eq!(stdout, concat!(
        "# HELP http_requests_total Requests\\\\total\\nnow\n",
        "# TYPE http_requests_total counter\n",
        "http_requests_total{route_name=\"GET \\\"/x\\\"\\n\"} 3\n",
        "# HELP http_latency Latency\n",
        "# TYPE http_latency histogram\n",
        "http_latency_bucket{route=\"/x\",le=\"10\"} 0\n",
        "http_latency_bucket{route=\"/x\",le=\"50\"} 1\n",
        "http_latency_bucket{route=\"/x\",le=\"+Inf\"} 1\n",
        "http_latency_sum{route=\"/x\"} 12\n",
        "http_latency_count{route=\"/x\"} 1\n",
        "\n",
    ));
}

#[test]
fn prometheus_normalization_collisions_are_rejected() {
    let output = run_certo(r#"module HostPrometheusCollisionTest
fn main(): Unit = {
    val host = Host.new()
    val first = Host.gaugeMetric(host, "queue-depth", "one", "items", [], 1)
    val second = Host.gaugeMetric(host, "queue.depth", "two", "items", [], 1)
}
"#);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("metric names collide after Prometheus normalization"), "{stderr}");
}

#[test]
fn prometheus_derived_and_label_collisions_are_rejected() {
    let derived = run_certo(r#"module HostPrometheusDerivedCollisionTest
fn main(): Unit = {
    val host = Host.new()
    val histogram = Host.histogramMetric(host, "request", "latency", "ms", [], [10], 1)
    val gauge = Host.gaugeMetric(host, "request_sum", "sum", "ms", [], 1)
}
"#);
    assert!(!derived.status.success());
    assert!(
        String::from_utf8_lossy(&derived.stderr)
            .contains("metric names collide after Prometheus normalization"),
        "{}",
        String::from_utf8_lossy(&derived.stderr),
    );

    let labels = run_certo(r#"module HostPrometheusLabelCollisionTest
fn main(): Unit = {
    val host = Host.new()
    val gauge = Host.gaugeMetric(host, "queue", "depth", "items", ["route-name", "route.name"], 1)
}
"#);
    assert!(!labels.status.success());
    assert!(
        String::from_utf8_lossy(&labels.stderr)
            .contains("label names collide after Prometheus normalization"),
        "{}",
        String::from_utf8_lossy(&labels.stderr),
    );
}

#[test]
fn concurrent_worker_events_have_one_total_sequence_order() {
    let output = run_certo(r#"module HostConcurrentEventTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(c)
    var i = 0
    while i < 20 {
        HostContext.log(c, "info", "worker.tick", intToText(i))
        i = i + 1
    }
    Ok(())
}
fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("workers", start, stop)
        .worker("one", work).worker("two", work))
    val started = Host.start(host)
    val stopped = Host.stop(host)
}
"#);
    assert_success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines = stderr.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 40, "{stderr}");
    for (index, line) in lines.iter().enumerate() {
        assert!(line.contains(&format!("\"sequence\":{}", index + 1)), "{line}");
        assert!(line.contains("\"worker\":"), "{line}");
    }
}

#[test]
fn compatibility_log_rejects_unknown_severity_before_emission() {
    let output = run_certo(r#"module HostInvalidSeverityTest
fn start(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.log(c, "notice", "job.started", "no event should be written")
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("invalid", start, stop))
    val started = Host.start(host)
}
"#);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid host log severity"), "{stderr}");
    assert!(!stderr.contains("certo.host.event/v1"), "invalid event was emitted: {stderr}");
}

fn redact_stress_artifact(bytes: &[u8], canary: Option<&str>) -> Vec<u8> {
    let Some(canary) = canary.filter(|value| !value.is_empty()) else {
        return bytes.to_vec();
    };
    String::from_utf8_lossy(bytes)
        .replace(canary, "[REDACTED]")
        .into_bytes()
}

fn write_stress_artifacts(
    path: &Path,
    source: &str,
    seed: i64,
    iterations: i64,
    sanitizer: &str,
    exit_status: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> std::io::Result<()> {
    let canary = env::var("CERTO_HOST_STRESS_SECRET_CANARY").ok();
    let safe = |bytes: &[u8]| redact_stress_artifact(bytes, canary.as_deref());
    fs::create_dir_all(path)?;
    fs::write(path.join("host_randomized_lifecycle_stress.cto"), safe(source.as_bytes()))?;
    fs::write(path.join("stdout.log"), safe(stdout))?;
    fs::write(path.join("stderr.log"), safe(stderr))?;
    fs::write(
        path.join("metadata.txt"),
        safe(format!(
            "seed={seed}\niterations={iterations}\nsanitizer={sanitizer}\nexit_status={exit_status}\n"
        ).as_bytes()),
    )?;
    fs::write(
        path.join("reproduce.sh"),
        safe(format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nCERTO_HOST_STRESS_SEED={seed} CERTO_HOST_STRESS_ITERATIONS={iterations} CERTO_HOST_STRESS_SANITIZER={sanitizer} cargo test -p certo --test host_integration randomized_lifecycle_stress -- --ignored --nocapture\n"
        ).as_bytes()),
    )?;
    Ok(())
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
fn randomized_stress_artifacts_capture_reproduction_context() {
    let dir = tempfile::tempdir().expect("temporary artifact directory");
    let source = HOST_RANDOMIZED_STRESS_SOURCE
        .replace("__SEED__", "41")
        .replace("__ITERATIONS__", "17");
    write_stress_artifacts(
        dir.path(),
        &source,
        41,
        17,
        "thread",
        "exit code: 66",
        b"partial stdout\n",
        b"WARNING: ThreadSanitizer: data race\n",
    )
    .expect("write synthetic failure artifacts");

    let saved_source = fs::read_to_string(dir.path().join("host_randomized_lifecycle_stress.cto"))
        .expect("read generated source artifact");
    assert!(saved_source.contains("var state = 41"), "{saved_source}");
    assert!(
        saved_source.contains("while iteration < 17"),
        "{saved_source}"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("metadata.txt")).unwrap(),
        "seed=41\niterations=17\nsanitizer=thread\nexit_status=exit code: 66\n"
    );
    assert_eq!(
        fs::read(dir.path().join("stdout.log")).unwrap(),
        b"partial stdout\n"
    );
    assert_eq!(
        fs::read(dir.path().join("stderr.log")).unwrap(),
        b"WARNING: ThreadSanitizer: data race\n"
    );
    let reproduce = fs::read_to_string(dir.path().join("reproduce.sh")).unwrap();
    assert!(reproduce.starts_with("#!/usr/bin/env bash\nset -euo pipefail\n"));
    assert!(reproduce.contains("CERTO_HOST_STRESS_SEED=41"));
    assert!(reproduce.contains("CERTO_HOST_STRESS_ITERATIONS=17"));
    assert!(reproduce.contains("CERTO_HOST_STRESS_SANITIZER=thread"));
    assert!(reproduce.contains(
        "cargo test -p certo --test host_integration randomized_lifecycle_stress -- --ignored --nocapture"
    ));
}

#[test]
fn sanitizer_artifact_redaction_removes_configured_canaries() {
    let secret = "sanitizer-secret-canary-7751";
    let artifact = redact_stress_artifact(
        format!("before {secret} after\n").as_bytes(), Some(secret));
    assert_eq!(artifact, b"before [REDACTED] after\n");
    assert!(!String::from_utf8_lossy(&artifact).contains(secret));
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
    let artifact_dir = env::var_os("CERTO_HOST_STRESS_ARTIFACT_DIR").map(PathBuf::from);
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_randomized_lifecycle_stress.cto");
    fs::write(&source_path, &source).expect("write Certo source");

    let mut command = Command::new(env!("CARGO_BIN_EXE_certo"));
    command.arg("run").arg(&source_path);
    let sanitizer = env::var("CERTO_HOST_STRESS_SANITIZER").unwrap_or_else(|_| "none".to_owned());
    match sanitizer.as_str() {
        "address" => {
            command.arg("--sanitize-address").env("CC", "clang").env(
                "ASAN_OPTIONS",
                "detect_leaks=0:halt_on_error=1:abort_on_error=1",
            );
        }
        "thread" => {
            command
                .arg("--sanitize-thread")
                .env("CC", "clang")
                .env("TSAN_OPTIONS", "halt_on_error=1:abort_on_error=1");
        }
        "none" => {}
        other => panic!("unknown CERTO_HOST_STRESS_SANITIZER: {other}"),
    }
    let output = command
        .output()
        .expect("run randomized host lifecycle stress test");
    if let Some(path) = &artifact_dir {
        fs::write(path.join("stdout.log"), &output.stdout).expect("write stress stdout artifact");
        fs::write(path.join("stderr.log"), &output.stderr).expect("write stress stderr artifact");
        fs::write(
            path.join("metadata.txt"),
            format!(
                "seed={seed}\niterations={iterations}\nsanitizer={sanitizer}\nexit_status={}\n",
                output.status
            ),
        )
        .expect("write stress metadata artifact");
        fs::write(
            path.join("reproduce.sh"),
            format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nCERTO_HOST_STRESS_SEED={seed} \\\n+CERTO_HOST_STRESS_ITERATIONS={iterations} \\\n+CERTO_HOST_STRESS_SANITIZER={sanitizer} \\\n+cargo test -p certo --test host_integration randomized_lifecycle_stress -- --ignored --nocapture\n"
            ),
        )
        .expect("write stress reproduction command artifact");
        write_stress_artifacts(
            path,
            &source,
            seed,
            iterations,
            &sanitizer,
            &output.status.to_string(),
            &output.stdout,
            &output.stderr,
        )
        .expect("write randomized stress artifacts");
    }
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

#[test]
fn typed_snapshots_cover_startup_restart_and_shutdown_transitions() {
    let startup = assert_success(&run_certo(
        r#"module HostStartupSnapshotTest
fn slowStart(c: HostContext): Result<Unit, Text> [io] = {
    sleep(40)
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn startHost(host: Host): Unit [io] = {
    val result = Host.startTyped(host)
    ()
}
fn observe(host: Host): Unit [io] = {
    sleep(5)
    val status = Host.status(host)
    println(HostState.name(HostStatusSnapshot.state(status)))
    println(boolToText(HostStatusSnapshot.isReady(status)))
    println(boolToText(HostStatusSnapshot.isLive(status)))
}
fn main(): Unit [io, async] = {
    val host = Host.new().add(Host.plugin("slow", slowStart, stop))
    val completed = await parallel { startHost(host), observe(host) }
    val stopped = Host.stopTyped(host)
}
"#,
    ));
    assert_eq!(startup.lines().collect::<Vec<_>>(), ["Starting", "false", "true"]);

    let restart = assert_success(&run_certo(
        r#"module HostRestartSnapshotTest
fn attemptsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("attempts")
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, attemptsKey()) {
        Some(attempts) => match Channel.tryReceive(attempts) {
            Some(_) => Err("transient")
            None => {
                HostContext.ready(c)
                while HostContext.sleep(c, Duration.milliseconds(1)) {}
                Ok(())
            }
        }
        None => Err("missing attempts")
    }
}
fn startHost(host: Host): Unit [io] = {
    val result = Host.startTyped(host)
    ()
}
fn observe(host: Host): Unit [io] = {
    sleep(20)
    val status = Host.status(host)
    println(HostState.name(HostStatusSnapshot.state(status)))
    match List.first(HostStatusSnapshot.workers(status)) {
        Some(worker) => {
            println(HostWorkerState.name(HostWorkerStatus.state(worker)))
            println(boolToText(HostWorkerStatus.isReady(worker)))
            println(boolToText(HostWorkerStatus.isLive(worker)))
            println(intToText(HostWorkerStatus.restarts(worker)))
        }
        None => println("missing worker")
    }
}
fn main(): Unit [io, async] = {
    val attempts = Channel.new(capacity: 1)
    Channel.send(attempts, 1)
    val host = Host.new().provide(attemptsKey(), attempts)
        .add(Host.plugin("plugin", start, stop).worker("worker", work)
            .restart(RestartPolicy.onFailure(2, Duration.milliseconds(100), Duration.milliseconds(100))))
    val completed = await parallel { startHost(host), observe(host) }
    val stopped = Host.stopTyped(host)
}
"#,
    ));
    assert_eq!(
        restart.lines().collect::<Vec<_>>(),
        ["Starting", "Restarting", "false", "true", "1"]
    );

    let shutdown = assert_success(&run_certo(
        r#"module HostShutdownSnapshotTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn quiesce(c: HostContext): Result<Unit, Text> [io] = {
    sleep(40)
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn stopHost(host: Host): Unit [io] = {
    val result = Host.stopTyped(host)
    ()
}
fn observe(host: Host): Unit [io] = {
    sleep(5)
    val status = Host.status(host)
    println(HostState.name(HostStatusSnapshot.state(status)))
    println(boolToText(HostStatusSnapshot.isReady(status)))
    println(boolToText(HostStatusSnapshot.isLive(status)))
}
fn main(): Unit [io, async] = {
    val host = Host.new().add(Host.plugin("plugin", start, stop).quiesce(quiesce))
    val started = Host.startTyped(host)
    val completed = await parallel { stopHost(host), observe(host) }
    val finalStatus = Host.status(host)
    println(HostState.name(HostStatusSnapshot.state(finalStatus)))
    println(boolToText(HostStatusSnapshot.isLive(finalStatus)))
}
"#,
    ));
    assert_eq!(
        shutdown.lines().collect::<Vec<_>>(),
        ["Stopping", "false", "true", "Stopped", "false"]
    );
}

#[test]
fn typed_lifecycle_calls_are_deterministic_when_repeated_and_concurrent() {
    let stdout = assert_success(&run_certo(
        r#"module HostTypedConcurrencyTest
fn start(c: HostContext): Result<Unit, Text> [io] = {
    sleep(25)
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn startOne(host: Host): Unit [io] = {
    match Host.startTyped(host) {
        Ok(_) => println("start-ok")
        Err(error) => println(HostFailureKind.name(HostLifecycleError.kind(error)))
    }
}
fn stopOne(host: Host): Unit [io] = {
    match Host.stopTyped(host) {
        Ok(_) => println("stop-ok")
        Err(error) => println(HostLifecycleError.message(error))
    }
}
fn main(): Unit [io, async] = {
    val host = Host.new().add(Host.plugin("plugin", start, stop))
    println(HostState.name(HostStatusSnapshot.state(Host.status(host))))
    val starts = await parallel { startOne(host), startOne(host) }
    println(HostState.name(HostStatusSnapshot.state(Host.status(host))))
    val stops = await parallel { stopOne(host), stopOne(host), stopOne(host), stopOne(host) }
    match Host.stopTyped(host) {
        Ok(_) => println("repeat-stop-ok")
        Err(error) => println(HostLifecycleError.message(error))
    }
    match Host.startTyped(host) {
        Ok(_) => println("unexpected restart")
        Err(error) => println(HostFailureKind.name(HostLifecycleError.kind(error)))
    }
}
"#,
    ));
    assert_eq!(stdout.lines().filter(|line| *line == "start-ok").count(), 1, "{stdout}");
    assert_eq!(stdout.lines().filter(|line| *line == "StartupFailure").count(), 2, "{stdout}");
    assert_eq!(stdout.lines().filter(|line| *line == "stop-ok").count(), 4, "{stdout}");
    assert_eq!(stdout.lines().filter(|line| *line == "New").count(), 1, "{stdout}");
    assert!(stdout.lines().any(|line| line == "Healthy"), "{stdout}");
    assert!(stdout.lines().any(|line| line == "repeat-stop-ok"), "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}

#[test]
fn typed_status_snapshot_exposes_host_workers_and_metrics() {
    let stdout = assert_success(&run_certo(
        r#"module HostTypedStatusTest

fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> [io] = {
    HostContext.counter(c, "jobs.started", 1)
    HostContext.gauge(c, "jobs.active", 1)
    HostContext.ready(c)
    while HostContext.sleep(c, Duration.milliseconds(1)) {}
    Ok(())
}

fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("plugin", start, stop).worker("worker", work))
    match Host.startTyped(host) {
        Ok(_) => {
            val status = Host.status(host)
            println(HostState.name(HostStatusSnapshot.state(status)))
            println(boolToText(HostStatusSnapshot.isReady(status)))
            println(boolToText(HostStatusSnapshot.isLive(status)))
            println(intToText(HostStatusSnapshot.workerCount(status)))
            println(intToText(HostStatusSnapshot.readyWorkers(status)))
            println(intToText(List.len(HostStatusSnapshot.counters(status))))
            println(intToText(List.len(HostStatusSnapshot.gauges(status))))
            match List.first(HostStatusSnapshot.workers(status)) {
                Some(worker) => {
                    println(HostWorkerStatus.name(worker))
                    println(HostWorkerStatus.plugin(worker))
                    println(HostWorkerState.name(HostWorkerStatus.state(worker)))
                    println(boolToText(HostWorkerStatus.isReady(worker)))
                    println(boolToText(HostWorkerStatus.isLive(worker)))
                    println(intToText(HostWorkerStatus.restarts(worker)))
                }
                None => println("missing worker")
            }
            match Host.stopTyped(host) {
                Ok(_) => {
                    val stopped = Host.status(host)
                    println(HostState.name(HostStatusSnapshot.state(stopped)))
                    match List.first(HostStatusSnapshot.workers(stopped)) {
                        Some(worker) => println(HostWorkerState.name(HostWorkerStatus.state(worker)))
                        None => println("missing stopped worker")
                    }
                }
                Err(error) => println(HostLifecycleError.message(error))
            }
        }
        Err(error) => println(HostLifecycleError.message(error))
    }
}
"#,
    ));

    let lines: Vec<_> = stdout.lines().collect();
    assert!(lines.starts_with(&[
        "Healthy", "true", "true", "1", "1", "2", "2", "worker", "plugin",
        "Healthy", "true", "true", "0",
    ]), "{stdout}");
    assert!(lines.ends_with(&["Stopped", "Stopped"]), "{stdout}");
    assert!(!stdout.contains("missing worker"), "{stdout}");
}

#[test]
fn typed_lifecycle_error_and_snapshot_preserve_failure_details() {
    let stdout = assert_success(&run_certo(
        r#"module HostTypedFailureTest

fn failStart(c: HostContext): Result<Unit, Text> = Err("database unavailable")
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn printFailure(error: HostLifecycleError): Unit [io] = {
    println(HostFailureKind.name(HostLifecycleError.kind(error)))
    println(HostLifecycleError.phase(error))
    println(HostLifecycleError.subject(error))
    println(HostLifecycleError.message(error))
    println(boolToText(HostLifecycleError.isTimeout(error)))
}

fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("database", failStart, stop))
    match Host.startTyped(host) {
        Ok(_) => println("unexpected success")
        Err(error) => printFailure(error)
    }
    val status = Host.status(host)
    println(HostState.name(HostStatusSnapshot.state(status)))
    println(boolToText(HostStatusSnapshot.isReady(status)))
    println(boolToText(HostStatusSnapshot.isLive(status)))
    match HostStatusSnapshot.lastFailure(status) {
        Some(error) => println(HostFailureKind.name(HostLifecycleError.kind(error)))
        None => println("missing failure")
    }
}
"#,
    ));

    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "StartupFailure", "start", "database", "database unavailable", "false",
            "Failed", "false", "false", "StartupFailure",
        ],
        "{stdout}"
    );
}

#[test]
fn typed_lifecycle_errors_classify_timeout_worker_and_shutdown_failures() {
    let timeout = assert_success(&run_certo(
        r#"module HostTypedTimeoutTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> [io] = {
    while HostContext.sleep(c, Duration.milliseconds(1)) {}
    Ok(())
}
fn main(): Unit [io] = {
    val host = Host.new().readinessTimeout(Duration.milliseconds(5))
        .add(Host.plugin("plugin", start, stop).worker("worker", work))
    match Host.startTyped(host) {
        Ok(_) => println("unexpected")
        Err(error) => {
            println(HostFailureKind.name(HostLifecycleError.kind(error)))
            println(HostLifecycleError.phase(error))
            println(boolToText(HostLifecycleError.isTimeout(error)))
        }
    }
}
"#,
    ));
    assert_eq!(timeout.lines().collect::<Vec<_>>(), ["Timeout", "readiness", "true"]);

    let worker = assert_success(&run_certo(
        r#"module HostTypedWorkerFailureTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn work(c: HostContext): Result<Unit, Text> = {
    HostContext.ready(c)
    Err("worker crashed")
}
fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("plugin", start, stop).worker("worker", work))
    match Host.runTyped(host) {
        Ok(_) => println("unexpected")
        Err(error) => {
            println(HostFailureKind.name(HostLifecycleError.kind(error)))
            println(HostLifecycleError.subject(error))
            println(HostLifecycleError.message(error))
        }
    }
}
"#,
    ));
    assert_eq!(
        worker.lines().collect::<Vec<_>>(),
        ["WorkerFailure", "worker", "worker crashed"]
    );

    let shutdown = assert_success(&run_certo(
        r#"module HostTypedShutdownFailureTest
fn start(c: HostContext): Result<Unit, Text> = Ok(())
fn failStop(c: HostContext): Result<Unit, Text> = Err("flush failed")
fn main(): Unit [io] = {
    val host = Host.new().add(Host.plugin("plugin", start, failStop))
    match Host.startTyped(host) {
        Ok(_) => match Host.stopTyped(host) {
            Ok(_) => println("unexpected")
            Err(error) => {
                println(HostFailureKind.name(HostLifecycleError.kind(error)))
                println(HostLifecycleError.phase(error))
                println(HostLifecycleError.subject(error))
                println(HostLifecycleError.message(error))
            }
        }
        Err(error) => println(HostLifecycleError.message(error))
    }
}
"#,
    ));
    assert_eq!(
        shutdown.lines().collect::<Vec<_>>(),
        ["ShutdownFailure", "stop", "plugin", "flush failed"]
    );
}

#[test]
fn stop_during_startup_cancels_and_waits_for_rollback() {
    let stdout = assert_success(&run_certo(
        r#"module HostStartupCancellationTest

fn slowStart(c: HostContext): Result<Unit, Text> [io] = {
    sleep(25)
    println("startup callback finished")
    Ok(())
}

fn stop(c: HostContext): Result<Unit, Text> [io] = {
    println("plugin stopped")
    Ok(())
}

fn startHost(host: Host): Unit [io] = {
    match Host.startTyped(host) {
        Ok(_) => println("unexpected startup success")
        Err(error) => {
            println(HostFailureKind.name(HostLifecycleError.kind(error)))
            println(HostLifecycleError.message(error))
        }
    }
}

fn stopHost(host: Host): Unit [io] = {
    sleep(1)
    match Host.stop(host) {
        Ok(_) => println("startup cancellation completed")
        Err(error) => println(f"unexpected stop error: {error}")
    }
}

fn main(): Unit [io, async] = {
    val host = Host.new().add(Host.plugin("slow", slowStart, stop))
    val completed = await parallel {
        startHost(host),
        stopHost(host),
    }
    println(Host.health(host))
}
"#,
    ));

    assert!(stdout.contains("host startup cancelled"), "{stdout}");
    assert!(stdout.contains("StartupCancelled"), "{stdout}");
    assert!(
        stdout.contains("startup cancellation completed"),
        "{stdout}"
    );
    assert!(stdout.contains("Failed"), "{stdout}");
    assert_eq!(
        stdout
            .lines()
            .filter(|line| *line == "plugin stopped")
            .count(),
        1,
        "{stdout}"
    );
    assert!(!stdout.contains("unexpected"), "{stdout}");
}

#[test]
fn service_factories_construct_before_plugins_and_dispose_in_reverse_order() {
    let stdout = assert_success(&run_certo(
        r#"module HostServiceFactoryTest

fn eventsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("events")
type Resource = { value: Int }
fn firstKey(): ServiceKey<Resource> = Host.serviceKey("first")
fn secondKey(): ServiceKey<Resource> = Host.serviceKey("second")

fn emit(c: HostContext, value: Int): Unit [io] = {
    match HostContext.service(c, eventsKey()) {
        Some(events) => Channel.send(events, value)
        None => ()
    }
}

fn createFirst(c: HostContext): Result<Resource, Text> [io] = {
    emit(c, 1)
    Ok(Resource { value: 10 })
}
fn disposeFirst(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, firstKey()) {
        Some(value) => {
            emit(c, value.value + 90)
            Ok(())
        }
        None => Err("first service unavailable during disposal")
    }
}
fn createSecond(c: HostContext): Result<Resource, Text> [io] = {
    match HostContext.service(c, firstKey()) {
        Some(value) => {
            emit(c, 2)
            Ok(Resource { value: value.value + 10 })
        }
        None => Err("first service unavailable")
    }
}
fn disposeSecond(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, secondKey()) {
        Some(value) => {
            emit(c, value.value + 180)
            Ok(())
        }
        None => Err("second service unavailable during disposal")
    }
}
fn start(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, secondKey()) {
        Some(_) => {
            emit(c, 3)
            Ok(())
        }
        None => Err("second service unavailable")
    }
}
fn stop(c: HostContext): Result<Unit, Text> [io] = {
    emit(c, 4)
    Ok(())
}

fn main(): Unit [io] = {
    val events = Channel.new(capacity: 8)
    val host = Host.new()
        .provide(eventsKey(), events)
        .provideFactory(secondKey(), createSecond, disposeSecond)
        .factoryDependsOn(secondKey(), firstKey())
        .provideFactory(firstKey(), createFirst, disposeFirst)
        .add(Host.plugin("plugin", start, stop))
    val started = Host.start(host)
    val stopped = Host.stop(host)
    for i in 0..5 {
        match Channel.tryReceive(events) {
            Some(value) => println(intToText(value))
            None => println("missing event")
        }
    }
}
"#,
    ));

    assert_eq!(stdout.lines().collect::<Vec<_>>(), ["1", "2", "3", "4", "200", "100"]);
}

#[test]
fn service_factory_failure_disposes_already_constructed_services() {
    let stdout = assert_success(&run_certo(
        r#"module HostServiceFactoryRollbackTest

type Resource = { value: Int }
fn eventsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("events")
fn firstKey(): ServiceKey<Resource> = Host.serviceKey("first")
fn failingKey(): ServiceKey<Resource> = Host.serviceKey("failing")
fn emit(c: HostContext, value: Int): Unit [io] = {
    match HostContext.service(c, eventsKey()) {
        Some(events) => Channel.send(events, value)
        None => ()
    }
}
fn createFirst(c: HostContext): Result<Resource, Text> [io] = {
    emit(c, 1)
    Ok(Resource { value: 10 })
}
fn disposeFirst(c: HostContext): Result<Unit, Text> [io] = {
    match HostContext.service(c, firstKey()) {
        Some(value) => {
            emit(c, value.value + 90)
            Ok(())
        }
        None => Err("first service unavailable during rollback")
    }
}
fn failCreate(c: HostContext): Result<Resource, Text> [io] = {
    emit(c, 2)
    Err("factory unavailable")
}
fn disposeFailing(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val events = Channel.new(capacity: 4)
    val host = Host.new()
        .provide(eventsKey(), events)
        .provideFactory(firstKey(), createFirst, disposeFirst)
        .provideFactory(failingKey(), failCreate, disposeFailing)
    match Host.start(host) {
        Ok(_) => println("unexpected success")
        Err(error) => println(error)
    }
    for i in 0..2 {
        match Channel.tryReceive(events) {
            Some(value) => println(intToText(value))
            None => println("missing event")
        }
    }
}
"#,
    ));

    assert!(stdout.lines().next().unwrap_or_default().contains("failed to construct service"), "{stdout}");
    assert!(stdout.lines().next().unwrap_or_default().contains("factory unavailable"), "{stdout}");
    assert_eq!(stdout.lines().skip(1).collect::<Vec<_>>(), ["1", "2", "100"], "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}

#[test]
fn service_factory_dependency_validation_runs_before_factories() {
    let stdout = assert_success(&run_certo(
        r#"module HostServiceFactoryDependencyValidationTest

type Resource = { value: Int }
fn eventsKey(): ServiceKey<Channel<Int>> = Host.serviceKey("events")
fn firstKey(): ServiceKey<Resource> = Host.serviceKey("first")
fn secondKey(): ServiceKey<Resource> = Host.serviceKey("second")
fn missingKey(): ServiceKey<Resource> = Host.serviceKey("missing")
fn create(c: HostContext): Result<Resource, Text> [io] = {
    match HostContext.service(c, eventsKey()) {
        Some(events) => Channel.send(events, 1)
        None => ()
    }
    Ok(Resource { value: 1 })
}
fn dispose(c: HostContext): Result<Unit, Text> = Ok(())
fn printStart(host: Host): Unit [io] = {
    match Host.start(host) {
        Ok(_) => println("unexpected success")
        Err(error) => println(error)
    }
}

fn main(): Unit [io] = {
    val events = Channel.new(capacity: 4)
    val missing = Host.new()
        .provide(eventsKey(), events)
        .provideFactory(firstKey(), create, dispose)
        .factoryDependsOn(firstKey(), missingKey())
    printStart(missing)

    val cyclic = Host.new()
        .provide(eventsKey(), events)
        .provideFactory(firstKey(), create, dispose)
        .provideFactory(secondKey(), create, dispose)
        .factoryDependsOn(firstKey(), secondKey())
        .factoryDependsOn(secondKey(), firstKey())
    printStart(cyclic)
    match Channel.tryReceive(events) {
        Some(_) => println("unexpected factory callback")
        None => println("no factories ran")
    }
}
"#,
    ));

    let lines = stdout.lines().collect::<Vec<_>>();
    assert!(lines[0].contains("missing factory dependency"), "{stdout}");
    assert!(lines[1].contains("dependencies form a cycle"), "{stdout}");
    assert_eq!(lines[2], "no factories ran", "{stdout}");
    assert!(!stdout.contains("unexpected"), "{stdout}");
}
