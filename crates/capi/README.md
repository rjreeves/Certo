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
C# project.

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
