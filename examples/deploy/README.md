# Deploying a Certo hosted application

These templates target the operational listener in `examples/host_probes.cto`.
Build that application as `certo-host` before constructing an image or copying
the binary to `/opt/certo-host`.

## Shutdown deadline

The service manager's grace period must exceed the application's worst-case
cooperative shutdown duration. Include listener drain, plugin quiesce, worker
drain, plugin stop, service disposal, and telemetry finalization, plus process
scheduling margin. Some phases run once per registered component, so adding
plugins, services, or sinks can increase the bound. The templates use 90
seconds as an example, not as a universal guarantee.

## systemd

Install `systemd/certo-host.service`, create the unprivileged `certo-host` user,
and place environment values in `/etc/certo-host/environment`. systemd sends
SIGTERM and waits for `TimeoutStopSec`; `Host.runTyped` maps that signal to the
typed `Signal` stop reason and completes cooperative shutdown.

## Containers and Kubernetes

The image declares SIGTERM as its stop signal. Docker Compose and Kubernetes
must grant enough time before SIGKILL. Kubernetes uses separate liveness,
readiness, and startup probes; readiness closes as soon as shutdown begins.
Replace the example image reference before applying the manifest.

The example image installs `curl` for the Compose health check. Kubernetes uses
native HTTP probes and does not invoke it.

## Windows Service Control Manager

A Certo console executable does not implement the native Windows `ServiceMain`
contract and must not be registered directly with `sc.exe`. Run it through a
service wrapper or a small native service adapter. Configure the wrapper's stop
hook to execute `windows/Stop-CertoHost.ps1`, wait for the process to exit, and
set its external kill deadline above the calculated host bound. Store the admin
token in the service account's protected configuration and do not put it on a
command line or in logs.
