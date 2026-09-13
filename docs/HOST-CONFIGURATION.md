# Host configuration contract

This document defines the H3 configuration contract. The typed configuration
pipeline, external source layering, and secret-bearing key protections are implemented.
Precedence, validation timing, diagnostics, and secret handling are normative.

## Configuration snapshot

`Host.start` builds one immutable configuration snapshot before constructing a
service or invoking a plugin callback. Every factory, plugin, and worker in one
host lifecycle observes that same snapshot. Source files and environment
variables are never read lazily from `HostContext`.

H3 intentionally does not reload configuration. Source changes take effect only
after constructing and starting a new host, normally through a process restart.
This keeps every callback in one lifecycle on the same immutable snapshot and
avoids adding watcher, rebinding, and partial-update semantics to the initial
contract. Reload may be proposed as a future milestone; any such design must
publish a complete new snapshot atomically and must never mutate an existing
snapshot in place.

## Deterministic precedence

Sources are merged from lowest to highest precedence:

1. typed defaults registered by the application;
2. the `[host]` subtree in `certo.toml`;
3. environment variables;
4. command-line configuration arguments;
5. programmatic overrides registered with `Host.configure`.

The highest source containing a key wins. A child key overrides only that exact
key, not its siblings. Within environment variables, command-line arguments,
and programmatic overrides, the last occurrence wins. Duplicate keys in one
TOML document are invalid rather than order-dependent.

Keys use lower-camel dotted paths such as `database.poolSize`. Key comparison is
case-sensitive after source-specific normalization. Environment variables use
the `CERTO__` prefix and double underscores for path separators, so
`CERTO__DATABASE__POOL_SIZE` normalizes to `database.poolSize` through the
registered typed key rather than an inferred spelling algorithm. Command-line
overrides use `--config key=value`; a missing value or malformed key is a
startup error.

Application keys live below `[host]`; for example `[host.database]` plus
`poolSize = "10"` supplies `database.poolSize`. Other manifest sections belong
to the compiler toolchain and are not Host configuration sources.

The default `certo.toml` is optional when absent. Once a file is explicitly
configured, absence, unreadability, malformed TOML, or an unsupported value
shape is a startup error. Relative paths are resolved from the process working
directory captured when the host is created.

## Typed binding and validation

Applications register typed keys before startup. A key owns:

- its canonical dotted name;
- a text parser returning `Result<T, Text>`;
- whether it is required or has a typed default;
- zero or more validators returning `Result<Unit, Text>`;
- whether its result structurally contains `Secret<_>`.

The typed foundation API is:

```certo
Host.configKey<T>(name: Text, parse: fn(Text): Result<T, Text>): ConfigKey<T>
Host.requireConfig<T>(host: Host, key: ConfigKey<T>): Host
Host.defaultConfig<T>(host: Host, key: ConfigKey<T>, value: T): Host
Host.validateConfig<T>(host: Host, key: ConfigKey<T>, validator: fn(T): Result<Unit, Text>): Host
HostContext.configValue<T>(context: HostContext, key: ConfigKey<T>): T
```

`HostContext.config` and `configOr` remain compatibility accessors for raw text.
They read the winning snapshot and do not bypass startup validation.

Binding is a preflight phase after source merging and before service dependency
validation. Registered keys are processed in registration order; validators for
one key run in registration order. All binding errors are collected into one
stable list ordered by key registration, then validator registration. If any
error exists, startup fails and no factory, plugin, worker, or disposer runs.

A required key with no winning value is invalid. A parser failure or validator
failure is invalid. Defaults are already typed and are validated without being
converted to text and parsed again. Unregistered raw keys remain available to
the compatibility accessors but do not participate in typed validation.

## Diagnostics

Configuration failures use a typed `ConfigurationFailure` lifecycle kind and
phase `configuration`. Each item reports:

- canonical key name;
- source kind: `Default`, `Toml`, `Environment`, `CommandLine`,
  `Programmatic`, or `None` for a missing required value;
- non-secret source location, such as a TOML path and line, environment variable
  name, argument index, or programmatic registration index;
- category: `Missing`, `Parse`, `Validation`, `Duplicate`, `Io`, or `Syntax`;
- a stable human-readable message.

The typed diagnostic surface is:

```certo
HostLifecycleError.configurationErrors(error: HostLifecycleError): List<HostConfigurationError>
HostConfigurationError.key(error): Text
HostConfigurationError.source(error): HostConfigurationSource
HostConfigurationError.location(error): Text
HostConfigurationError.category(error): HostConfigurationErrorKind
HostConfigurationError.message(error): Text
```

`HostConfigurationSource` has `Default`, `Toml`, `Environment`, `CommandLine`,
and `Programmatic` variants. `HostConfigurationErrorKind` has `Missing`,
`Parse`, `Validation`, `Duplicate`, `Io`, and `Syntax` variants. Non-
configuration lifecycle errors return an empty configuration-error list.

Diagnostics never include the raw or parsed value. Aggregated text errors follow
the same stable ordering as typed errors. Source attribution describes where a
value came from, not its contents.

## Secrets

A `ConfigKey<Secret<T>>`, or any key whose result structurally contains
`Secret<_>`, is secret-bearing. The compiler's existing `Secret<_>` sink rules
continue to reject logging and serialization of the bound value.

The host marks secret-bearing keys from their parsed type at compile time. It
replaces parser and validator failure text for those keys with stable redacted
messages. The host additionally guarantees that secret-bearing raw text and
parsed values never appear in logs, metrics, status snapshots, lifecycle
messages, panic messages, or sanitizer failure artifacts. Parser and validator
callbacks for a secret-bearing key may return descriptive errors, but the host
never appends their returned text. Source names and locations are safe to report.

Raw compatibility lookup of a registered secret-bearing key fails at runtime; callers
must use its typed `ConfigKey`. This prevents `HostContext.config` from turning a
protected secret back into unrestricted `Text`.

## Lifecycle and concurrency

Configuration sources, keys, defaults, and validators can be registered only
while the host is `New`. Snapshot construction has no concurrent callbacks.
After successful preflight, reads are immutable and safe from every host
callback. A startup cancellation requested during source loading or validation
finishes the current bounded operation, publishes `StartupCancelled`, and does
not begin service construction.

## Initial implementation acceptance tests

- Every precedence pair is tested, including repeated CLI and programmatic
  entries.
- Missing, parse, and validation failures occur before callbacks and have stable
  typed ordering.
- All callbacks observe the same immutable snapshot under concurrency.
- Secret values are absent from text errors, structured logs, metrics, status,
  and generated failure artifacts.
- Windows and Linux normalize registered environment keys identically.
