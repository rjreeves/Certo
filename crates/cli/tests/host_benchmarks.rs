use std::{collections::BTreeMap, env, fs, process::Command};

const BENCHMARK_SOURCE: &str = r#"module HostObservabilityBenchmark

fn writeEvent(event: HostLogEvent): Result<Unit, Text> = Ok(())
fn flushSink(): Result<Unit, Text> = Ok(())
fn disposeSink(): Result<Unit, Text> = Ok(())
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn emit(c: HostContext, count: Int): Unit [io] = {
    var i = 0
    while i < count {
        HostContext.log(c, "info", "benchmark.event", "payload")
        i = i + 1
    }
}

fn start(c: HostContext): Result<Unit, Text> [io, async] = {
    val started = monotonicMillis()
    val completed = await parallel {
        emit(c, __EVENTS_PER_PRODUCER__),
        emit(c, __EVENTS_PER_PRODUCER__),
        emit(c, __EVENTS_PER_PRODUCER__),
        emit(c, __EVENTS_PER_PRODUCER__),
    }
    println(f"admission_ms={monotonicMillis() - started}")
    Ok(())
}

fn update(counter: HostMetric, gauge: HostMetric, histogram: HostMetric, count: Int): Unit [io] = {
    var i = 0
    while i < count {
        HostMetric.counterAdd(counter, ["shared"], 1)
        HostMetric.gaugeSet(gauge, ["shared"], i)
        HostMetric.histogramObserve(histogram, ["shared"], i % 1000)
        i = i + 1
    }
}

fn main(): Unit [io, async] = {
    val host = Host.new()
        .disableStderrLog()
        .telemetryTimeout(Duration.seconds(30))
        .logSink("benchmark", __SINK_CAPACITY__, HostLogOverflowPolicy.wait(Duration.seconds(5)),
            HostLogFailurePolicy.failHost(), writeEvent, flushSink, disposeSink)
        .add(Host.plugin("producer", start, stop))
    val counter = Host.counterMetric(host, "benchmark.counter", "Counter", "updates", ["key"], 4)
    val gauge = Host.gaugeMetric(host, "benchmark.gauge", "Gauge", "value", ["key"], 4)
    val histogram = Host.histogramMetric(host, "benchmark.histogram", "Histogram", "ms", ["key"], [10, 100, 500, 1000], 4)

    val dispatchStarted = monotonicMillis()
    val started = Host.start(host)
    val stopped = Host.stop(host)
    println(f"dispatch_ms={monotonicMillis() - dispatchStarted}")

    val updateStarted = monotonicMillis()
    val updates = await parallel {
        update(counter, gauge, histogram, __UPDATES_PER_PRODUCER__),
        update(counter, gauge, histogram, __UPDATES_PER_PRODUCER__),
        update(counter, gauge, histogram, __UPDATES_PER_PRODUCER__),
        update(counter, gauge, histogram, __UPDATES_PER_PRODUCER__),
    }
    println(f"updates_ms={monotonicMillis() - updateStarted}")

    val snapshotStarted = monotonicMillis()
    var snapshotIteration = 0
    while snapshotIteration < __EXPORT_ITERATIONS__ {
        val snapshot = Host.metricSnapshot(host)
        snapshotIteration = snapshotIteration + 1
    }
    println(f"snapshots_ms={monotonicMillis() - snapshotStarted}")

    val exportStarted = monotonicMillis()
    var exportIteration = 0
    while exportIteration < __EXPORT_ITERATIONS__ {
        val output = Host.metricsPrometheus(host)
        exportIteration = exportIteration + 1
    }
    println(f"prometheus_ms={monotonicMillis() - exportStarted}")
}
"#;

const EVENTS_PER_PRODUCER: i64 = 25_000;
const UPDATES_PER_PRODUCER: i64 = 10_000;
const EXPORT_ITERATIONS: i64 = 1_000;

fn threshold(name: &str, default_ms: i64) -> i64 {
    let key = format!("CERTO_HOST_BENCH_{}_MAX_MS", name.to_ascii_uppercase());
    env::var(&key)
        .map(|value| {
            value
                .parse()
                .unwrap_or_else(|_| panic!("{key} must be an integer"))
        })
        .unwrap_or(default_ms)
}

#[test]
#[ignore = "scheduled performance regression gate"]
fn observability_contention_stays_within_regression_thresholds() {
    let source = BENCHMARK_SOURCE
        .replace("__EVENTS_PER_PRODUCER__", &EVENTS_PER_PRODUCER.to_string())
        .replace("__SINK_CAPACITY__", &(EVENTS_PER_PRODUCER * 4).to_string())
        .replace(
            "__UPDATES_PER_PRODUCER__",
            &UPDATES_PER_PRODUCER.to_string(),
        )
        .replace("__EXPORT_ITERATIONS__", &EXPORT_ITERATIONS.to_string());
    let dir = tempfile::tempdir().expect("temporary benchmark directory");
    let source_path = dir.path().join("host_observability_benchmark.cto");
    fs::write(&source_path, source).expect("write benchmark source");

    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .output()
        .expect("run native host observability benchmark");
    assert!(
        output.status.success(),
        "benchmark failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("benchmark output is UTF-8");
    let measured = stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| {
            (
                name.to_owned(),
                value
                    .parse::<i64>()
                    .unwrap_or_else(|_| panic!("invalid benchmark result: {name}={value}")),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let limits = [
        ("admission_ms", threshold("admission", 3_000)),
        ("dispatch_ms", threshold("dispatch", 5_000)),
        ("updates_ms", threshold("updates", 3_000)),
        ("snapshots_ms", threshold("snapshots", 1_000)),
        ("prometheus_ms", threshold("prometheus", 1_000)),
    ];
    for (name, maximum) in limits {
        let actual = measured
            .get(name)
            .unwrap_or_else(|| panic!("missing {name} in benchmark output:\n{stdout}"));
        assert!(
            *actual <= maximum,
            "{name} regression: {actual}ms exceeded {maximum}ms\n{stdout}"
        );
    }
    eprintln!(
        "host observability benchmark: events={} metric_updates={} exports={} {}",
        EVENTS_PER_PRODUCER * 4,
        UPDATES_PER_PRODUCER * 4 * 3,
        EXPORT_ITERATIONS,
        stdout.trim().replace('\n', " "),
    );
}
