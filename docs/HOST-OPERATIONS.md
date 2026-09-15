# Host operational endpoints and control contract

This document defines the H5 operational endpoint, coordinated-control, and
deployment contract. `Stdlib.Http` adapters translate typed host facts into
transport-specific responses, while applications retain control of exposure
and authentication.

## Endpoint model

The standard adapter will expose three read-only operations:

- **liveness** reports whether the process can still make lifecycle progress;
- **readiness** reports whether new application traffic may be admitted; and
- **metrics** returns the deterministic Prometheus snapshot defined by H4.

An optional authenticated administrative operation requests coordinated drain.
Authentication and authorization belong to the application or HTTP adapter;
the host never creates an unauthenticated control endpoint by default.

## State mapping

| Host state | Live | Ready | Operational condition |
|---|---:|---:|---|
| New | yes | no | starting |
| Starting | yes | no | starting |
| Healthy | yes | yes | ready |
| Starting after a worker restart | yes | no | degraded |
| Stopping | yes | no | stopping |
| Stopped | no | no | stopped |
| Failed | no | no | failed |

Readiness closes before plugin quiesce callbacks begin. Once closed during
shutdown it cannot reopen. Liveness remains true while bounded drain and stop
work can make progress, preventing a service manager from killing a process
that is still completing its declared shutdown contract.

## Coordinated drain

A signal, administrative request, or `Host.requestStop` selects one shutdown
owner. The owner closes readiness and new telemetry admission, quiesces plugins,
drains workers, stops plugins, disposes scoped services, and finalizes telemetry
under the existing phase deadlines. Concurrent requests await and reuse the
owner's typed result.

## Start and stop reasons

`Host.startReason` records why the process instance was created. It defaults to
`HostStartReason.application()` and may be set to `serviceManager`, `restart`,
or `testRun` only while the host is new. The typed value is available through
`HostOperationalStatus.startReason` and is rendered as `Application`,
`ServiceManager`, `Restart`, or `Test`.

`HostOperationalStatus.stopReason` is absent until an accepted shutdown cause
or terminal lifecycle failure occurs. Its stable names are
`ApplicationRequest`, `AdministrativeDrain`, `Signal`, `StartupFailure`, and
`WorkerFailure`. The first cause wins under concurrent requests. A rejected
stop of a host that has never started does not create a stop reason. Raw error
messages are never used as reasons.

The control adapter must acknowledge only that the request was accepted. It
must not keep the request open until shutdown finishes, and it must not invent a
second timeout policy. Repeated requests are idempotent.

## Response requirements

Probe responses use a stable typed snapshot before transport encoding. They
contain host state, live/ready flags, operational condition, and a sanitized
failure category when present. They never contain configuration values, raw
callback errors, metric label values, or plugin-owned payloads.

## Standard HTTP response adapters

`HostHttp.liveness(host)` and `HostHttp.readiness(host)` return deterministic
JSON containing `condition`, `state`, `live`, `ready`, and `start_reason`, plus
`stop_reason` when present. They return status
200 when the selected predicate is true and 503 otherwise. A failed host also
includes the stable `failure_kind`; raw failure messages are excluded.

`HostHttp.metrics(host)` returns status 200 and the current Prometheus text
snapshot with content type `text/plain; version=0.0.4; charset=utf-8`.
Applications retain control of paths, routing, authentication, and whether any
adapter is exposed. The adapters perform no socket I/O themselves.

`HostHttp.serve` registers a named listener owned by the host. Binding occurs
during startup. Shutdown closes its listening socket before plugin quiesce,
joins the accept loop, and waits for active request callbacks only until the
listener's declared drain timeout. A timeout becomes a typed shutdown failure
with phase `http-drain`.

`HostHttp.drain` requests graceful shutdown and returns 202 immediately.
Applications must authenticate and authorize the request before calling it;
the standard library never publishes an administrative route automatically.

## Worker maintenance controls

`Host.disableWorker(host, name)` cooperatively cancels a single worker using
the same `HostContext.isStopping`, `sleep`, and `waitUntil` boundary used during
host shutdown. The supervisor remains owned by the host in a parked `Disabled`
state. A disabled worker is excluded from both required-worker and ready-worker
counts, so planned maintenance does not make the otherwise healthy host
unready.

`Host.enableWorker(host, name)` resumes that supervisor and closes readiness
until the callback reports ready again. Repeated operations are idempotent.
Both APIs return `HostWorkerControlStatus`, whose stable values are `Disabled`,
`Enabled`, `AlreadyDisabled`, `AlreadyEnabled`, `UnknownWorker`,
`InvalidHostState`, `ControlInProgress`, and `Timeout`. Disable uses the host's
worker shutdown timeout; a timeout leaves the disable request in force and does
not terminate the native thread.
