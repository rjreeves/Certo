# Stdlib.Host Roadmap

`Stdlib.Host` is Certo's in-process application host for composing services,
plugins, and long-running background workers. Its design is inspired by managed
application hosts such as .NET Generic Host, while preserving Certo's static
typing, explicit effects, and native deployment model.

This roadmap is the source of truth for future Host work. The language-wide
roadmap remains in `CERTO-SPEC.md`; completed implementation details remain in
`BACKLOG.md`.

## Product direction

The host should make a Certo executable a reliable container for internal
application components. An application should be able to register typed
services, declare plugin dependencies, run supervised workers, expose health
and telemetry, and shut down predictably without adopting a separate framework.

The host belongs in the standard library because lifecycle, cancellation,
signals, concurrency, logging, and service typing all depend on language and
runtime semantics. Keeping the core contract in the stdlib gives applications
one portable lifecycle model across supported native platforms.

The first production target is statically linked, trusted plugins compiled with
the application. Loading arbitrary third-party native libraries is a later,
explicit interoperability layer rather than a requirement for the core host.

## Status legend

- **Shipped** — implemented, documented, and covered by automated tests.
- **Next** — the recommended immediate milestone.
- **Planned** — intended after the next milestone; design may still change.
- **Exploratory** — useful direction that requires a design proposal first.

## Shipped foundation

### Composition and validation

- Typed `ServiceKey<T>` registration and lookup.
- Plugin-provided and required service declarations.
- Stable dependency ordering: providers start before consumers.
- Missing dependencies, duplicate providers, duplicate names, dependency
  cycles, invalid restart policies, and invalid telemetry names are rejected
  before startup.
- Host mutation is rejected after startup begins.

### Lifecycle

- Ordered plugin startup and reverse-order shutdown.
- Startup rollback for plugins that already started.
- Separate quiesce, worker-drain, and plugin-stop phases.
- Independent readiness, quiesce, drain, and stop timeouts.
- Idempotent concurrent shutdown.
- Ctrl+C, SIGTERM, and programmatic stop requests.
- Explicit host health states and worker health inspection.
- Startup-completion publication prevents shutdown from observing partially
  initialized worker handles.
- Shutdown during initial startup cancels startup, waits for rollback, and has a
  stable result contract; shutdown during worker restart remains graceful.

### Managed background workers

- Workers start after plugin initialization.
- Explicit worker readiness reporting.
- Shutdown-aware sleep and predicate waiting.
- `never`, `onFailure`, and `always` restart policies.
- Bounded exponential restart backoff.
- Restart counts and last-error inspection.
- Exhausted retries transition the host to failure.

### Observability

- Serialized structured JSON logging with host, plugin, and worker context.
- Named counters and gauges.
- Built-in lifecycle, readiness, restart, and failure metrics.
- JSON metrics snapshot through `Host.metrics`.

### Reliability verification

- Native lifecycle and failure-path integration tests.
- Registration-validation tests.
- Concurrent shutdown regression coverage.
- AddressSanitizer and ThreadSanitizer stress jobs.
- Nightly and manually dispatched randomized lifecycle stress with reproducible
  seeds and configurable iteration counts.
- Failure artifacts containing generated source, metadata, sanitizer output,
  and a reproduction command.

## Milestone H1 — Operational contracts

**Status: Shipped**

Turn the current runtime behavior into explicit, stable contracts before adding
more extension points.

### Scope

- ✅ Define a typed host status snapshot instead of requiring consumers to
  parse the `Host.metrics` JSON string.
- ✅ Define readiness and liveness semantics for the host and every worker.
- ✅ Distinguish startup failure, runtime worker failure, shutdown failure, and
  timeout in public diagnostics; reserve `ForcedTermination` for the future
  non-cooperative termination mechanism.
- ✅ Add typed lifecycle errors for startup cancellation and other operational
  failure categories while retaining the stable text compatibility API.
- ✅ Specify timeout boundaries and the state reported after each timeout.
- ✅ Document callback concurrency rules and which Host APIs are safe inside
  each callback phase.
- ✅ Add contract tests for every state transition and repeated/concurrent call.

### Exit criteria

- The lifecycle state machine is documented as a table or diagram.
- Every public state transition has a deterministic result and regression test.
- No public operational API requires parsing a human-oriented error string.
- Native tests pass on Windows and Linux; ASan and TSan remain clean.

## Milestone H2 — Scoped services and disposal

**Status: Shipped**

Evolve the service registry from a typed value map into a lifecycle-aware
dependency container without hiding ownership.

### Scope

- ✅ Singleton factory registration for services requiring runtime construction.
- ✅ Deterministic reverse-order disposal for host-owned singleton services.
- ✅ Startup rollback disposes every successfully constructed service.
- ✅ Per-plugin factory scopes constructed before the owning plugin and disposed
  after it stops.
- ✅ Scoped services are visible only to their owning plugin and may depend on
  host services, but not another plugin's scope.
- ✅ Missing, duplicate, and cyclic factory dependencies fail before callbacks
  run; factories use stable topological construction order.
- ✅ Explicit rules for borrowing, sharing, and thread safety, including bounded
  disposal and retention after cooperative timeouts.

### Exit criteria

- Disposal order is deterministic and tested under failure and timeout paths.
- A service cannot be observed before construction or after disposal.
- Scope boundaries are represented by types or explicit APIs, not conventions.

## Milestone H3 — Configuration and secrets

**Status: Next**

Provide one typed configuration pipeline suitable for local development,
testing, and production deployment.

### Scope

- Layered configuration from defaults, `certo.toml`, environment variables,
  command-line arguments, and programmatic overrides.
- Typed binding and validation at host startup.
- Source attribution for diagnostics without exposing secret values.
- Integration with Certo's `Secret<T>` protections.
- Optional configuration reload with an immutable snapshot model.

### Exit criteria

- Precedence and reload behavior are deterministic and documented.
- Invalid configuration prevents plugin startup with field-level diagnostics.
- Secrets never appear in logs, metrics, snapshots, or error artifacts.

## Milestone H4 — Production observability

**Status: Planned**

Keep the current zero-dependency local telemetry while adding export interfaces
for production systems.

### Scope

- Structured event schema with stable field names and severity levels.
- Pluggable log sinks with backpressure and failure policy.
- Metrics registry with counter, gauge, and histogram instruments.
- Prometheus text export.
- OpenTelemetry-compatible traces and metrics after a separate design review.
- Correlation context propagated through plugin and worker callbacks.

### Exit criteria

- Exporters cannot block host shutdown indefinitely.
- Telemetry failures have an explicit drop, buffer, or fail policy.
- High-contention logging and metrics paths have stress and benchmark coverage.

## Milestone H5 — Operational endpoints and control

**Status: Planned**

Make hosted applications straightforward to operate in service managers and
container platforms.

### Scope

- Standard liveness, readiness, and metrics adapters for `Stdlib.Http`.
- Coordinated drain triggered by signals or an administrative endpoint.
- Startup and shutdown reason reporting.
- Optional worker enable/disable controls for maintenance.
- Platform integration guidance for systemd, Windows Service Control Manager,
  and containers.

### Exit criteria

- Kubernetes-style probes can distinguish alive, ready, degraded, and stopping.
- A deployment can drain traffic before worker cancellation.
- Service-manager stop deadlines map predictably to host timeouts.

## Milestone H6 — Plugin packaging and compatibility

**Status: Exploratory**

Define a versioned plugin contract before supporting separately distributed
components.

### Scope

- Plugin manifest containing identity, version, capabilities, and dependencies.
- Host and plugin API compatibility policy.
- Compile-time discovery for packages included in the application build.
- Duplicate capability and incompatible version diagnostics.
- Deterministic activation and isolation boundaries.

### Exit criteria

- A manifest schema and compatibility policy are accepted before implementation.
- Statically packaged plugins require no unsafe runtime type casts.
- Dependency resolution produces a reproducible build graph.

## Milestone H7 — Dynamic and out-of-process plugins

**Status: Exploratory**

Only pursue dynamic loading after the static contract is stable and a concrete
deployment need justifies the additional security and compatibility surface.

### Possible tracks

- Versioned C ABI for trusted native dynamic libraries.
- WASM component boundary for portable sandboxed plugins.
- Out-of-process plugins over a versioned protocol for crash isolation.

Each track requires an independent proposal covering ABI stability, capability
security, resource limits, crash recovery, upgrades, and observability. Dynamic
loading must not weaken the type safety of the ordinary static Host API.

## Cross-cutting quality gates

Every Host milestone must satisfy the following gates:

- Windows and Linux behavior is equivalent or differences are documented.
- Lifecycle changes include native integration tests.
- Concurrent changes include deterministic race tests and TSan coverage.
- Native allocation changes include ASan coverage.
- Failures preserve a reproducible seed, source, and diagnostic logs.
- Public APIs are documented in `STDLIB-QUICKREF.md` with a complete example.
- Breaking behavior includes a migration note and compatibility decision.
- Benchmarks guard startup, shutdown, logging, and metrics overhead where the
  change affects a hot path.

## Explicit non-goals for the current host

- Untrusted in-process native plugins.
- Transparent distributed dependency injection.
- Automatic recovery from arbitrary memory corruption.
- Hidden global service lookup outside a `HostContext`.
- Unbounded retries or shutdown waits.
- A promise of stable dynamic-library ABI before H6 is complete.

## Recommended next implementation item

Begin H3 with a design for deterministic layered configuration precedence,
typed binding, validation diagnostics, and secret-safe source attribution.
