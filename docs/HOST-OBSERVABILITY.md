# Host production observability contract

This document defines the H4 contract for structured events, log sinks,
metrics, correlation, and telemetry shutdown. It is normative for H4
implementation work. OpenTelemetry transport and trace semantics require a
separate design review and are not defined here.

## Compatibility baseline

The host remains useful without an exporter or third-party dependency.
`HostContext.log` continues to emit serialized JSON lines to stderr by default,
and `Host.metrics` continues to return a JSON snapshot. Existing counter and
gauge names and built-in lifecycle metrics retain their meaning.

New typed APIs feed the same event and metric registries as the compatibility
APIs. Installing a custom sink does not silently remove stderr output; the
application must explicitly disable the default sink before startup.

## Structured event schema

Every accepted event is an immutable value with these stable fields:

| Field | Type | Required | Meaning |
|---|---|---:|---|
| `schema` | `Text` | yes | Schema identifier, initially `certo.host.event/v1`. |
| `sequence` | `Int` | yes | Host-local, monotonically increasing acceptance sequence. |
| `timestamp_unix_ms` | `Int` | yes | UTC Unix time when the host accepts the event. |
| `severity` | `HostLogSeverity` | yes | `Trace`, `Debug`, `Info`, `Warn`, `Error`, or `Fatal`. |
| `event` | `Text` | yes | Stable machine-readable event name. |
| `message` | `Text?` | no | Human-readable explanation, not an identity or grouping key. |
| `host_id` | `Text` | yes | Stable identity for one host instance. |
| `plugin` | `Text?` | no | Plugin identity inherited from the callback context. |
| `worker` | `Text?` | no | Worker identity inherited from the callback context. |
| `correlation_id` | `Text?` | no | Current correlation identity, when present. |
| `fields` | `List<HostLogField>` | yes | Ordered typed fields; empty when none were supplied. |

Event names and field names use the same restricted identifier grammar as
metric names. Duplicate field names are rejected before dispatch. A field is
one of `Text`, `Int`, `Float`, `Bool`, or `Duration`; exporters must preserve
its type. Unknown schema fields may be ignored, but existing fields cannot be
reinterpreted within `v1`.

`HostContext.log(level, event, message)` is a compatibility adapter. Level
matching is ASCII case-insensitive; the six canonical names map to
`HostLogSeverity`, while any other level is rejected. It creates an event with
no user fields and inherits context exactly like the typed API.

## Ordering and concurrency

The host assigns `sequence` while accepting an event. Accepted events have one
total host-local order even when plugins and workers log concurrently. Each
sink observes accepted events in increasing sequence order, except events
discarded by its declared overflow policy. Different sinks may make progress
independently and must not share a queue or failure state.

Event construction and queue admission are thread-safe. A sink callback is
serial: the host never invokes the same sink concurrently. Sink callbacks may
not mutate host registration or invoke lifecycle operations on their own host.

## Sink registration and ownership

Sinks are registered only while the host is `New`. A registration declares:

- a unique name;
- a positive bounded queue capacity;
- an overflow policy;
- a failure policy;
- an event callback;
- an optional flush callback; and
- a required disposer for resources owned by the sink.

The typed surface is designed around these values:

```certo
type HostLogOverflowPolicy = | DropNewest | DropOldest | Wait(Duration)
type HostLogFailurePolicy = | Ignore | Disable | FailHost

Host.logSink(
    host: Host,
    name: Text,
    capacity: Int,
    overflow: HostLogOverflowPolicy,
    failure: HostLogFailurePolicy,
    write: fn(HostLogEvent): Result<Unit, Text>,
    flush: (fn(): Result<Unit, Text>)?,
    dispose: fn(): Result<Unit, Text>
): Host

Host.disableStderrLog(host: Host): Host
Host.telemetryTimeout(host: Host, timeout: Duration): Host
HostContext.logEvent(context: HostContext, event: HostLogEvent): Unit [io]
```

Exact constructor helpers may evolve during implementation, but registration
validation must prevent an unbounded capacity or invalid policy combination
from reaching startup. Duplicate names, zero or negative capacity, invalid
timeouts, and callbacks missing required effects fail registration validation
before plugins start.

The host owns a registered sink from successful registration until disposal.
Startup failure disposes initialized sinks. Normal shutdown stops admission,
drains or expires each queue, flushes, and disposes sinks in reverse
registration order. Disposal runs once even after write or flush failure.

## Overflow and failure policy

Queues are always bounded. Overflow is deterministic per sink:

- `DropNewest` discards the event being admitted.
- `DropOldest` discards the oldest queued event and admits the new event.
- `Wait(duration)` waits only for the declared duration, then discards the new
  event. Shutdown interrupts the wait.

Host lifecycle threads never wait without a deadline. The default stderr sink
is synchronous and has no queue; its writes remain serialized.

Every discarded event increments
`host.telemetry.events_dropped_total{sink,reason}` internally. The initial
metrics API may expose the label tuple through a normalized metric name until
typed labels ship. The host must not recursively log a sink overflow.

A callback error applies the registered failure policy:

- `Ignore` records the failure metric and continues dispatching later events.
- `Disable` records the failure, disables that sink, discards its queued events,
  and still flushes and disposes it during shutdown.
- `FailHost` records a typed telemetry failure and requests orderly shutdown.

Sink errors are never sent back through the failing sink. They remain visible
through host status, built-in metrics, and other healthy sinks. Error text must
not contain the rejected event payload.

## Telemetry shutdown

`Host.telemetryTimeout` bounds the combined drain, flush, and disposal work for
each sink; the default is 10 seconds. Sinks are processed independently, so one
expired sink does not prevent later sinks from being finalized. A timeout is a
typed shutdown failure with phase `telemetry`, subject equal to the sink name,
and `isTimeout = true`.

Concurrent `Host.stop*` callers reuse the single owner of telemetry shutdown in
the same way they reuse plugin and service shutdown. No exporter may extend the
host's stop deadline indefinitely. Events attempted after admission closes are
dropped with reason `shutdown`; they cannot reopen a sink or block shutdown.

## Metrics registry

H4 adds explicit counter, gauge, and histogram descriptors. A descriptor has a
unique name, help text, unit, instrument kind, and an ordered list of label
names. Re-registering an identical descriptor is idempotent; reusing a name
with a different descriptor is a startup error.

- Counters are signed 64-bit storage but reject negative increments.
- Gauges accept signed 64-bit replacement values.
- Histograms accept non-negative integer observations and fixed increasing
  bucket boundaries registered before startup. Bucket counts are cumulative.
- A label set must exactly match the descriptor's declared labels.

The registry bounds cardinality per instrument. Exceeding the configured limit
drops a new label set, increments `host.telemetry.metric_series_dropped_total`,
and never evicts an existing series implicitly. Metric snapshots are immutable,
ordered by descriptor registration and then canonical label order, and safe to
read concurrently with updates.

The compatibility `counter` and `gauge` calls lazily create unlabeled
descriptors. Using one name as both kinds remains invalid.

## Prometheus export

`Host.metricsPrometheus` returns a point-in-time Prometheus text exposition from
the same registry as `Host.status` and `Host.metrics`. Export does no network
I/O and acquires no lock while formatting after the snapshot is copied.

Names are normalized deterministically to the Prometheus identifier grammar;
collisions after normalization are startup errors. Help text and label values
are escaped according to the Prometheus text format. Counters use a `_total`
suffix, histograms emit `_bucket`, `_sum`, and `_count`, and output ordering is
stable for tests and reproducible diagnostics.

## Correlation context

Every `HostContext` carries optional immutable correlation context. Plugin and
worker contexts inherit it automatically. An application may derive a context
with a new correlation identity for downstream work; derivation does not mutate
the parent context.

```certo
Host.correlation(host: Host, correlationId: Text): Host
HostContext.withCorrelation(context: HostContext, correlationId: Text): HostContext
HostContext.hostId(context: HostContext): Text
HostContext.plugin(context: HostContext): Text?
HostContext.worker(context: HostContext): Text?
HostContext.correlationId(context: HostContext): Text?
```

The root identity is configured before startup and is copied into host-service,
plugin-scoped, plugin, and worker callback contexts. Deriving a context changes
only the correlation identity; host, plugin, worker, configuration, services,
shutdown state, and readiness ownership remain the same. Worker restarts reuse
their inherited root identity.

Correlation identity flows into structured events and may be selected as an
explicit metric label, but it is never added to metrics automatically because
that would create unbounded cardinality. Propagation across HTTP or other
protocol boundaries belongs to the corresponding adapter milestone.

## Secret safety

Typed log fields structurally containing `Secret<_>` are rejected by the same
compiler sink rule as other logging APIs. The host never copies raw
configuration values into events, metric labels, status, exporter errors, or
telemetry failure artifacts. Sink diagnostics contain sink identity and failure
category, not event content.

Explicitly exposing a secret converts it back to its underlying type and is an
application-level trust decision; exporters cannot reconstruct its provenance.
Sink implementations must treat accepted events as sensitive operational data
and must not echo an event payload in their own error text.

As a defense in depth measure, the host retains the winning raw values for
registered secret-bearing configuration keys. If an explicitly exposed value
reaches a human-readable event or callback failure message, every occurrence is
replaced with `[REDACTED]`. Secret values used as event identities, correlation
identities, metric names, metadata, or label values are rejected with a stable
diagnostic that does not echo the value. Consequently status snapshots,
compatibility errors, typed lifecycle failures, JSON metrics, and Prometheus
output cannot reproduce a registered secret value.

Sanitizer stress artifact generation applies the same canary-redaction policy
through `CERTO_HOST_STRESS_SECRET_CANARY` before writing captured source,
stdout, stderr, metadata, or reproduction instructions.

## Acceptance tests

H4 implementation is complete when automated tests demonstrate:

- exact `v1` schema fields and compatibility-log mapping;
- total per-host sequencing and per-sink ordering under concurrent producers;
- every overflow and callback-failure policy without recursive telemetry;
- bounded drain, flush, and disposal on success, failure, and timeout;
- shutdown reuse by concurrent callers;
- counter, gauge, histogram, label, collision, and cardinality behavior;
- deterministic JSON and Prometheus snapshots;
- correlation inheritance and isolation; and
- compiler and runtime secret-leak prevention across events, metrics, status,
  failures, and generated sanitizer artifacts.

ASan, TSan, and Windows stress jobs cover the queue, registry, and shutdown
paths. Benchmarks track event admission, dispatch throughput, metric update,
snapshot, and exporter-formatting costs under low and high contention.

The scheduled release benchmark uses four concurrent producers to admit
100,000 events into a bounded sink and four concurrent metric producers to
perform 120,000 counter, gauge, and histogram mutations. It also repeats typed
snapshot and Prometheus formatting 1,000 times. The default regression ceilings
are 3 seconds for admission, 5 seconds through final dispatch, 3 seconds for
metric updates, and 1 second each for snapshots and Prometheus export. Dedicated
performance environments may override these with
`CERTO_HOST_BENCH_<PHASE>_MAX_MS`; a result above the selected ceiling fails CI.
