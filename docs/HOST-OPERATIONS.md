# Host operational endpoints and control contract

This document begins the H5 design for operational endpoints and coordinated
control. It is intentionally a contract first: no networking dependency is
added to `Stdlib.Host`, and `Stdlib.Http` adapters will translate these typed
host facts into transport-specific responses.

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
JSON containing `condition`, `state`, `live`, and `ready`. They return status
200 when the selected predicate is true and 503 otherwise. A failed host also
includes the stable `failure_kind`; raw failure messages are excluded.

`HostHttp.metrics(host)` returns status 200 and the current Prometheus text
snapshot with content type `text/plain; version=0.0.4; charset=utf-8`.
Applications retain control of paths, routing, authentication, and whether any
adapter is exposed. The adapters perform no socket I/O themselves.
