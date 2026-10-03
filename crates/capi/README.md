# certo-capi

C ABI for the Certo database compiler (SDL, migration plans, SQL), so a host
such as the C# CLI can call it in-process.

| File | Purpose |
|------|---------|
| `include/certo.h` | The C contract: signatures, ownership rules, JSON result shape |
| `bindings/CertoNative.cs` | P/Invoke binding for .NET (checks the ABI version on load) |
| `src/api.rs` | All JSON shapes and error codes, as plain Rust functions |

## Build

```powershell
cargo build -p certo-capi --release
# -> target/release/certo_capi.dll  (libcerto_capi.so / .dylib elsewhere)
```

Copy the library next to the host executable and add `CertoNative.cs` to the
C# project, or use the NuGet package below.

## NuGet package (`Certo.Native`)

`packaging/dotnet/` builds one package with the binding and the native library for
`win-x64`, `linux-x64`, `linux-arm64`, `osx-x64` and `osx-arm64` (under
`runtimes/<rid>/native/`, so NuGet copies the right one next to the app).

```powershell
pwsh packaging/dotnet/stage-native.ps1            # build + stage this machine's library
dotnet pack packaging/dotnet/Certo.Native -c Release -o packaging/dotnet/feed
dotnet run --project packaging/dotnet/Certo.Native.Smoke -c Release   # tests the PACKED package
```

`.github/workflows/dotnet-package.yml` does this for every platform, packs them together,
smoke-tests the package on Windows, Linux (x64 and arm64) and macOS, and on a
`certo-native-v<version>` tag (or a manual run with `publish` ticked) pushes the package to this
repository's GitHub Packages feed, authenticated by the workflow's own token (see
`packaging/dotnet/Certo.Native/README.md` for installing from it). The tag and
`crates/capi/Cargo.toml` versions must match, and the smoke test checks the native library
reports the package's version. To release: bump `version` in `crates/capi/Cargo.toml`, merge,
then `git tag certo-native-v<version> && git push origin certo-native-v<version>`.

CI runs the Rust live tests, the packed-package smoke test and the reference CLI against a real PostgreSQL
(service container), so the runner's PostgreSQL path (enum values added in their own batch, drift, repair
scripts, adoption) is covered as well as SQLite's.

Self-contained by design: the Windows build links the C runtime statically, the Linux build
vendors OpenSSL (glibc 2.35+ is the only requirement), macOS uses system frameworks, and SQLite
is bundled. The smoke test covers every entry point, the SQLite runner (apply, drift, table
rebuild, adopt), compiled QL run through `Microsoft.Data.Sqlite`, and concurrent calls.

## Flow

```
CompileSdl(schema.sdl text)        -> ir (persist as IR.json), diagnostics
DiffIr(previous ir, new ir)        -> plan, summary[], destructive
LowerSql(plan, "postgres")         -> batches[], script
```

Every call returns a JSON object with `"ok"`. `ok: false` means either the
schema has errors (`diagnostics` array, with 1-based line/column in UTF-16
units to match .NET strings) or the call itself failed (`error.code`, e.g.
`invalid_ir`, `unsupported_version`, `unknown_dialect`, `unsupported`).

## Migration runner

```
MigrateInit(dir)                                   -> project files
MigrateNew(dir, {"name":..., "mdl":{...}})         -> frozen migration (reviewable SQL)
MigrateList(dir)                                   -> migrations on disk
MigrateStatus(dir, {"url":...})                    -> applied / pending
MigrateApply(dir, {"url":..., "dry_run":..., "to":..., "check_drift":...})
MigrateDrift(dir, {"url":...})                     -> findings + optional repair SQL
MigrateAdopt(dir, {"url":..., "dry_run":...})       -> schema.sdl + baseline from an existing database
```

Each call is self-contained (connect, work, disconnect), so there are no
handles to keep or free. Compile errors from `MigrateNew` carry positioned
`diagnostics` exactly like `CompileSdl`; a failed `MigrateApply` lists in
`error.applied` the migrations that had already succeeded. Options reject
unknown fields, and connection errors never echo the URL's password. See
`src/runner_api.rs` for every option, result field and error code, and
`../runner/README.md` for what each operation guarantees.

A reference host built on the typed API lives in `packaging/dotnet/Certo.Db.Cli` (`certo-db`: schema,
query, code generation and migration commands); see its README. It is the model for the C# CLI.

## Guarantees

- Returned strings are owned by the library; release them with
  `certo_string_free` (the C# binding does this for you).
- Every call runs on an internal 64 MB-stack thread, so the caller's stack
  size is irrelevant, and panics are caught and returned as `error.code:
  "panic"` instead of aborting the host.
- Functions are pure and thread-safe.
- `certo_abi_version()` must match `CERTO_ABI_VERSION`; bump it on any
  incompatible change to a signature or JSON shape.

## Executing batches

`batches[]` are meant to be run in order: a batch with `transactional: false`
runs each statement on its own (PostgreSQL enum `ADD VALUE`), a
`transactional: true` batch runs inside one transaction.
