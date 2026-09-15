# Static host plugin manifest and compatibility contract

This document defines H6's contract for trusted plugins compiled into the same
native application as `Stdlib.Host`. It does not permit runtime native-library
loading, remote package fetching, or untrusted code.

## Manifest

Each plugin package contains one `certo-plugin.json` conforming to
`schemas/host-plugin.schema.json`. Paths are relative to the manifest and must
remain within its package directory after canonicalization.

- `schema_version` versions the manifest syntax independently of APIs. Version
  1 readers reject every other value and every unknown field.
- `id` is a stable lowercase, namespaced identifier and is unique in a build.
- `version` is the plugin's semantic version.
- `host_api` is the half-open host API interval `[min, max_exclusive)`.
- `entry` identifies a Certo source file, its declared module, and a public
  zero-argument factory returning `HostPlugin`.
- `capabilities` declares versioned functionality supplied by the plugin.
- `dependencies` declares plugin IDs and accepted half-open version intervals;
  `optional` defaults to false.

Manifests contain no secrets, machine-specific absolute paths, executable
hooks, URLs, or native library names.

## Version and compatibility policy

Host API, plugin, and capability versions use SemVer. A host accepts a plugin
only when its own API version is greater than or equal to `host_api.min` and
strictly less than `host_api.max_exclusive`. Dependency versions use the same
half-open comparison. Pre-release versions compare by SemVer precedence;
build metadata does not affect compatibility.

For the Host API, a major increment may remove or change public contracts, a
minor increment may add backward-compatible APIs or typed values, and a patch
increment contains compatible fixes. Manifest schema changes are not inferred
from Host API versions.

The entry factory's fully inferred type must be exactly `fn(): HostPlugin`.
Plugin services and callbacks continue through ordinary Certo types; discovery
must never cast an untyped native pointer into a service or callback.

## Resolution and diagnostics

Resolution is entirely local and occurs before C generation:

1. Canonicalize and sort manifest paths by normalized UTF-8 path.
2. Parse each manifest with unknown-field rejection.
3. Validate identifiers, SemVer values, intervals (`min < max_exclusive`), and
   package-contained entry paths.
4. Reject duplicate plugin IDs and duplicate capability IDs.
5. Resolve required dependencies by ID and version; include an optional edge
   only when its target is present and compatible.
6. Reject cycles and topologically order providers before consumers, breaking
   otherwise-independent ties lexicographically by plugin ID.
7. Compile entry modules and verify factory signatures before invoking any
   factory or host callback.

Every diagnostic identifies the manifest path, JSON field, stable category,
and sanitized explanation. Resolution never selects one of multiple versions
implicitly: more than one manifest with the same plugin ID is a duplicate-ID
error even when versions differ. The selected manifest bytes and dependency
graph contribute to the build fingerprint, making identical inputs produce an
identical activation order.

## Compatibility boundary

The compatibility promise covers manifest v1, the declared Host API interval,
factory typing, capability identity, and deterministic resolution. It does not
promise a stable C ABI. All H6 plugins are source packages compiled with the
application's Certo compiler and linked statically. Dynamic libraries, WASM,
and out-of-process protocols remain separate H7 proposals.
