# Hosting with Certo

## Building reliable applications from small, typed plugins

**First edition — Certo Host API 1.x**

Hosting is the part of an application that remains after the business logic is
written: startup order, shared resources, configuration, background work,
health checks, telemetry, graceful shutdown, and deployment. Certo puts those
responsibilities behind a typed standard-library API so applications can use
one lifecycle model from a local executable to a production service.

This book develops that model incrementally. It assumes basic familiarity with
Certo functions, records, `Result`, and effect annotations, but no prior
experience with `Stdlib.Host`.

---

## Contents

1. [Why an application host?](#1-why-an-application-host)
2. [Your first hosted application](#2-your-first-hosted-application)
3. [The lifecycle contract](#3-the-lifecycle-contract)
4. [Services and dependency ordering](#4-services-and-dependency-ordering)
5. [Typed configuration](#5-typed-configuration)
6. [Host-managed background workers](#6-host-managed-background-workers)
7. [Failure handling and typed operations](#7-failure-handling-and-typed-operations)
8. [Structured logging and metrics](#8-structured-logging-and-metrics)
9. [Health, readiness, and operational HTTP](#9-health-readiness-and-operational-http)
10. [Packaging and discovering plugins](#10-packaging-and-discovering-plugins)
11. [Graceful shutdown](#11-graceful-shutdown)
12. [Testing and sanitizer stress](#12-testing-and-sanitizer-stress)
13. [Deployment patterns](#13-deployment-patterns)
14. [A complete design checklist](#14-a-complete-design-checklist)

---

## 1. Why an application host?

A command-line program can open a resource, do some work, and exit. A service
has a more demanding shape. Several components must start in a valid order,
share carefully owned resources, run until asked to stop, and then unwind in a
bounded and observable way.

Without a host, each application invents its own answers to questions such as:

- What happens when the third component fails after two have started?
- Who owns a database pool, and when is it disposed?
- How does a worker know shutdown has started?
- When should a readiness probe close?
- Can two concurrent stop requests run shutdown twice?
- How do logs, metrics, and errors avoid exposing secrets?

`Stdlib.Host` supplies one deterministic answer to each question. A host owns
plugins. Plugins own lifecycle callbacks, scoped services, and workers. The
host validates the entire composition before invoking application code.

The key idea is deliberately modest:

> A host is a typed owner of application lifetime, not a hidden global runtime.

Ordinary Certo programs do not need it. Hosted programs opt in explicitly with
`Host.new()`.

## 2. Your first hosted application

A plugin needs a name, a start callback, and a stop callback:

```certo
module HelloHost

import Stdlib.Host

fn start(ctx: HostContext): Result<Unit, Text> [io] = {
    println("hello plugin started")
    Ok(())
}

fn stop(ctx: HostContext): Result<Unit, Text> [io] = {
    println("hello plugin stopped")
    Ok(())
}

fn main(): Unit [io] = {
    val host = Host.new()
        .add(Host.plugin("hello", start, stop))

    match Host.runTyped(host) {
        Ok(_) => ()
        Err(error) => eprintln(HostLifecycleError.message(error))
    }
}
```

`Host.runTyped` starts the application, waits for a stop request or operating
system signal, and performs shutdown. For a short-lived workflow, an
application may call `Host.startTyped` and `Host.stopTyped` separately.

Use typed lifecycle methods in new applications. The older `Host.start`,
`Host.stop`, and `Host.run` methods return `Result<Unit, Text>` and remain useful
for compatibility, but typed failures preserve phase, subject, timeout state,
and failure category.

## 3. The lifecycle contract

The host has a small state machine:

```text
New -> Starting -> Healthy -> Stopping -> Stopped
                  \                    /
                   ------> Failed <---
```

Registration is allowed only in `New`. Startup validates configuration and
service dependencies before callbacks run. Plugins start in registration
order. They quiesce and stop in reverse order.

Reverse shutdown is important. If plugin B starts after and depends on plugin
A, B stops before A. The dependency remains available for the full lifetime of
its consumer.

Startup is transactional. If A and B start successfully but C fails, the host
stops B and then A before returning C's original failure. The application does
not have to build a parallel rollback mechanism.

Shutdown is single-owner. The first accepted caller performs the phases;
concurrent callers wait and reuse the same result. Calling `Host.stop` during
startup requests cancellation, waits for rollback, and never starts a second
shutdown sequence.

Every blocking phase has a deadline. Configure them before startup:

```certo
val host = Host.new()
    .readinessTimeout(Duration.seconds(15))
    .quiesceTimeout(Duration.seconds(10))
    .drainTimeout(Duration.seconds(20))
    .stopTimeout(Duration.seconds(10))
    .disposalTimeout(Duration.seconds(10))
    .telemetryTimeout(Duration.seconds(5))
```

## 4. Services and dependency ordering

Services make dependencies explicit and typed. A `ServiceKey<T>` names one
service of type `T`:

```certo
type Logger = { prefix: Text }

fn loggerKey(): ServiceKey<Logger> = Host.serviceKey("logger")

val host = Host.new()
    .provide(loggerKey(), Logger { prefix: "[orders]" })
```

Callbacks resolve services through their `HostContext`:

```certo
fn startOrders(ctx: HostContext): Result<Unit, Text> [io] =
    match HostContext.service(ctx, loggerKey()) {
        Some(logger) => {
            println(logger.prefix ++ " starting")
            Ok(())
        }
        None => Err("logger service unavailable")
    }
```

Factories are appropriate when construction needs configuration or another
service. The host records ownership and runs disposers exactly once:

```certo
type Client = { endpoint: Text }

fn clientKey(): ServiceKey<Client> = Host.serviceKey("api.client")

fn createClient(ctx: HostContext): Result<Client, Text> =
    Ok(Client { endpoint: HostContext.configOr(ctx, "api.endpoint", "http://localhost") })

fn disposeClient(ctx: HostContext): Result<Unit, Text> = Ok(())

val host = Host.new()
    .provideFactory(clientKey(), createClient, disposeClient)
```

Use `.factoryDependsOn(factoryKey, dependencyKey)` to declare factory ordering.
Use `.provides(key)` and `.requires(key)` on plugins to declare plugin-level
service relationships. Missing providers, duplicate providers, dependency
cycles, and type mismatches fail before startup.

Services registered on a plugin are scoped to that plugin. A plugin scope may
depend on host services, but it cannot reach another plugin's private scope.
This keeps ownership local and prevents accidental service-locator coupling.

## 5. Typed configuration

Configuration enters the host from five layers, lowest precedence first:

1. typed defaults;
2. the `[host]` subtree of `certo.toml`;
3. `CERTO__...` environment variables;
4. repeated `--config key=value` arguments; and
5. programmatic `Host.configure` overrides.

The host builds one immutable snapshot before constructing services or starting
plugins. Every callback in that lifecycle sees the same values.

Define a typed key with a parser:

```certo
fn parsePort(value: Text): Result<Int, Text> =
    match parseInt(value) {
        Some(port) => Ok(port)
        None => Err("expected an integer port")
    }

fn portKey(): ConfigKey<Int> = Host.configKey("http.port", parsePort)

fn validPort(port: Int): Result<Unit, Text> =
    if port > 0 and port < 65536 then Ok(())
    else Err("port must be between 1 and 65535")

val host = Host.new()
    .defaultConfig(portKey(), 8080)
    .validateConfig(portKey(), validPort)
```

Read the typed value in a callback:

```certo
val port = HostContext.configValue(ctx, portKey())
```

Required, parse, and validation failures are collected before application
callbacks begin. Typed diagnostics report the key, source, safe location,
category, and message in deterministic order.

Secret-bearing configuration uses `Secret<T>`. The compiler rejects secret
values at logging, metric, status, and serialization sinks. The host also
redacts registered secret values from callback failures and generated stress
artifacts. Do not convert a secret back to plain text until it reaches the
smallest trusted boundary that actually needs it.

## 6. Host-managed background workers

A worker belongs to a plugin and receives a `HostContext`:

```certo
fn consume(ctx: HostContext): Result<Unit, Text> [io] = {
    HostContext.ready(ctx)
    while HostContext.sleep(ctx, Duration.seconds(1)) {
        HostContext.counter(ctx, "orders.polls", 1)
    }
    Ok(())
}

val orders = Host.plugin("orders", startOrders, stopOrders)
    .worker("consumer", consume)
    .restart(RestartPolicy.onFailure(
        3,
        Duration.milliseconds(100),
        Duration.seconds(2)))
```

Workers begin after plugin initialization. A worker reports readiness exactly
once with `HostContext.ready`. Startup waits for required workers within the
host readiness deadline.

Cooperative waiting is essential. `HostContext.sleep` and
`HostContext.waitUntil` wake when shutdown or maintenance cancellation begins.
A worker should not hide inside an unbounded operating-system sleep or blocking
call that the host cannot interrupt.

Restart policies are bounded:

- `RestartPolicy.never()` leaves the failure terminal.
- `RestartPolicy.onFailure(...)` retries failed callbacks.
- `RestartPolicy.always(...)` also restarts callbacks that return success.

Maintenance controls can disable a worker without stopping the host:

```certo
val disabled = Host.disableWorker(host, "consumer")
val enabled = Host.enableWorker(host, "consumer")
```

A disabled worker is removed from required readiness counts. Enabling it closes
readiness until it reports ready again.

## 7. Failure handling and typed operations

A typed lifecycle error answers four operational questions:

- What kind of failure occurred?
- In which lifecycle phase?
- Which plugin, worker, service, listener, or sink was responsible?
- Was the deadline exceeded?

```certo
match Host.runTyped(host) {
    Ok(_) => ()
    Err(error) => {
        eprintln(HostFailureKind.name(HostLifecycleError.kind(error)))
        eprintln(HostLifecycleError.message(error))
    }
}
```

The first causal failure is retained deterministically. Later cleanup failures
are aggregated without replacing the original cause. Configuration failures
also expose a stable list through
`HostLifecycleError.configurationErrors(error)`.

Inside a worker, request orderly shutdown with `Host.requestStop(ctx)`.
Callbacks must not synchronously invoke start, run, or stop on their own host;
that would make a lifecycle callback wait on itself.

## 8. Structured logging and metrics

The compatibility logging API is intentionally small:

```certo
HostContext.log(ctx, "info", "orders.started", "consumer is ready")
```

Events receive a host-local sequence number, timestamp, host identity, plugin,
worker, and correlation context. Concurrent producers still create one total
acceptance order.

Typed events add ordered fields and explicit severity. Custom sinks have
bounded queues, deterministic overflow policies, failure policies, flush
callbacks, and disposal deadlines. Stderr remains enabled unless the
application explicitly calls `Host.disableStderrLog`.

Metrics support counters, gauges, and histograms with typed descriptors,
bounded label cardinality, and deterministic snapshots. Compatibility helpers
cover common unlabeled metrics:

```certo
HostContext.counter(ctx, "orders.processed", 1)
HostContext.gauge(ctx, "orders.inflight", 4)
```

Export the registry as JSON or Prometheus text:

```certo
val json = Host.metricSnapshot(host)
val prometheus = Host.metricsPrometheus(host)
```

Metric names are normalized for Prometheus, and normalization collisions fail
registration. Snapshot ordering is stable, which makes tests and operational
diffs reproducible.

Correlation starts at the host and flows into plugin and worker contexts:

```certo
val host = Host.new().correlation("deployment-2026-10-06")
```

Correlation IDs are never added to metrics automatically because doing so
would create unbounded cardinality.

## 9. Health, readiness, and operational HTTP

Liveness asks whether the process can still make lifecycle progress. Readiness
asks whether it should receive new application traffic. They are deliberately
different during startup, worker recovery, and shutdown.

The standard adapters return deterministic responses but do not choose routes
or expose sockets by themselves:

```certo
fn handler(request: HttpRequest): HttpResponse [io] = {
    val path = HttpRequest.path(request)
    if path == "/live" then HostHttp.liveness(host)
    else if path == "/ready" then HostHttp.readiness(host)
    else if path == "/metrics" then HostHttp.metrics(host)
    else Http.notFound("not found")
}
```

Register the listener under host ownership:

```certo
HostHttp.serve(host, "operations", 8080, handler, Duration.seconds(5))
```

The host binds the listener during startup and closes admission before plugin
quiesce. Active requests drain only until the declared timeout.

An administrative drain endpoint must be authenticated by the application.
After authorization, `HostHttp.drain(host)` requests coordinated shutdown and
returns `202` immediately. The standard library never exposes an unauthenticated
control route automatically.

## 10. Packaging and discovering plugins

Static plugin packages use `certo-plugin.json`:

```json
{
  "schema_version": 1,
  "id": "dev.example.orders",
  "version": "1.2.0",
  "host_api": {
    "min": "1.0.0",
    "max_exclusive": "2.0.0"
  },
  "entry": {
    "source": "plugin.cto",
    "module": "Orders",
    "factory": "ordersPlugin"
  },
  "capabilities": [
    { "id": "dev.example.order-processing", "version": "1.0.0" }
  ],
  "dependencies": []
}
```

The factory has one exact signature:

```certo
pub fn ordersPlugin(): HostPlugin =
    Host.plugin("orders", startOrders, stopOrders)
```

List manifests explicitly in `certo.toml`:

```toml
[host-plugins]
manifests = [
  "plugins/orders/certo-plugin.json",
  "plugins/reporting/certo-plugin.json",
]
```

Then opt into composition:

```certo
val host = Host.discovered(Host.new())
```

Plain `Host.new()` remains empty. `Host.discovered` is an identity operation
when no manifests are configured.

Discovery validates schema versions, semantic-version ranges, capabilities,
required and optional dependencies, cycles, entry containment, modules, and
factory signatures. Providers are composed before consumers with lexical
tie-breaking. Plugin declarations are isolated with deterministic prefixes, so
separate packages may reuse private type, value, helper, and factory names.

Every build emits `CERTO_HOST_PLUGIN_FINGERPRINT`, a SHA-256 digest over the
Host API version, resolved graph, entry metadata, capabilities, dependency
ranges and versions, and source bytes. Paths, manifest listing order, and JSON
formatting do not affect it. Verbose builds print both the canonical graph and
the fingerprint.

Static plugins are compiled into the application. There is no runtime download,
registry lookup, unsafe type cast, or dynamic-library ABI in the H6 contract.

## 11. Graceful shutdown

Shutdown is a sequence, not a signal handler:

1. retain the first stop reason;
2. close readiness and telemetry admission;
3. stop accepting HTTP connections;
4. invoke plugin quiesce callbacks in reverse order;
5. cooperatively cancel and drain workers;
6. stop plugins in reverse order;
7. dispose plugin-scoped and host services;
8. drain, flush, and dispose telemetry sinks; and
9. publish the terminal result to every waiting caller.

A quiesce callback should stop accepting new domain work while leaving the
resources needed to finish existing work intact. The stop callback runs later,
after workers have drained.

Choose deadlines from the outside in. A service manager's termination grace
period must be longer than the application's maximum coordinated shutdown.
Otherwise the operating system can kill the process while Certo is still
honouring its own contract.

## 12. Testing and sanitizer stress

Test the lifecycle as a set of failure paths, not only a happy path. Important
cases include:

- failure before any plugin starts;
- failure after several plugins start;
- stop requested during startup;
- simultaneous stop callers;
- workers becoming ready in different orders;
- worker failure during shutdown;
- a service disposer or telemetry sink timing out;
- dependency cycles and duplicate registrations; and
- secrets used as canaries in every diagnostic channel.

Certo's Host suite uses native integration tests plus AddressSanitizer and
ThreadSanitizer stress workflows. Randomized lifecycle tests retain their seed,
iteration, source, stdout, stderr, and reproduction command when they fail.

For local native stress:

```sh
cargo test -p certo --test host_integration
cargo test -p certo --test host_plugin_discovery
```

Sanitizer testing is most useful when the seed and iteration count are explicit
and preserved. A random test that cannot be reproduced is only an alarm; a
random test with a stable artifact is a debugging tool.

## 13. Deployment patterns

### systemd

Run the process in the foreground, use restart policies at the service-manager
level for process failures, and make `TimeoutStopSec` larger than the complete
host shutdown budget. Send `SIGTERM`; the host closes readiness and drains.

### Windows services

Translate Service Control Manager stop requests into the same host stop path.
Do not build a second shutdown implementation in the adapter. Report pending
status while the bounded host operation is still making progress.

### Docker and Kubernetes

Expose separate liveness and readiness routes. Use a pre-stop action or
authenticated administrative drain when the platform needs readiness to close
before sending its final signal. Set `terminationGracePeriodSeconds` above the
application's maximum shutdown duration.

Deployment templates live under `examples/deploy/`. Treat them as starting
points: ports, identities, resource limits, secret mounts, and authentication
remain application-specific.

## 14. A complete design checklist

Before shipping a hosted Certo application, verify the following.

### Composition

- Every plugin has a unique operational name.
- Dependencies use typed service keys.
- Factory and plugin dependency graphs are acyclic.
- Plugin manifests and Host API ranges are pinned intentionally.

### Lifecycle

- Startup failures roll back all started plugins.
- Workers use interruptible host waits.
- Quiesce stops admission before resources are disposed.
- Every phase has a realistic deadline.
- Concurrent stop requests reuse one result.

### Configuration and secrets

- Required values use typed keys.
- Validation occurs before factories or callbacks.
- Secret-bearing types never cross logging or metric sinks.
- Production configuration precedence is documented.

### Operations

- Liveness and readiness have separate routes.
- Administrative drain is authenticated.
- Prometheus output is scraped from the host registry.
- The service-manager deadline exceeds the host's shutdown budget.

### Verification

- Native success and failure paths have integration tests.
- Concurrent paths run under ThreadSanitizer.
- Allocation and disposal paths run under AddressSanitizer.
- Stress failures retain reproducible artifacts.
- The plugin graph fingerprint is recorded with the release.

---

## Further reading

- [Host standard-library reference](STDLIB-QUICKREF.md#host)
- [Host roadmap](HOST-ROADMAP.md)
- [Configuration contract](HOST-CONFIGURATION.md)
- [Observability contract](HOST-OBSERVABILITY.md)
- [Operations contract](HOST-OPERATIONS.md)
- [Plugin compatibility contract](HOST-PLUGIN-COMPATIBILITY.md)
- [Complete host example](../examples/host.cto)
- [Operational probe example](../examples/host_probes.cto)
- [Dropbox plugin and hosted workflow](../examples/dropbox_host.cto)

The host is in the standard library because application lifetime is a shared
language concern. Plugins should be able to compose without selecting a
framework, and the compiler should understand enough of the contract to reject
unsafe or ambiguous compositions before the process starts. `Stdlib.Host`
provides that common ground while keeping participation explicit.
