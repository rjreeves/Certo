# Certo.Native

The Certo database compiler, in-process. One static class, `Certo.CertoNative`; every call
takes and returns JSON text.

```csharp
using System.Text.Json;
using Certo;

var sdl = "table users { id: serial primary key  email: text not null unique }";
using var compiled = JsonDocument.Parse(CertoNative.CompileSdl(sdl));
var ir = compiled.RootElement.GetProperty("ir").GetRawText();

// QL: typed queries and mutations, for PostgreSQL or SQLite
var ql = CertoNative.CompileQl(ir, "query by_email(e: text) { from users u where u.email == :e select u.id }", "sqlite");

// Migrations: a project directory, then apply to a database
CertoNative.MigrateInit(dir, "{\"dialect\":\"sqlite\"}");
// write dir/schema.sdl, then:
CertoNative.MigrateNew(dir, "{\"name\":\"init\"}");
CertoNative.MigrateApply(dir, "{\"url\":\"app.db\"}");
```

`"ok": false` is a normal outcome (schema errors come back as positioned `diagnostics`);
only an ABI mismatch between the binding and the native library throws.

## Platforms

The package carries the native library for `win-x64`, `linux-x64`, `linux-arm64`,
`osx-x64` and `osx-arm64` under `runtimes/<rid>/native/`; NuGet copies the right one next to
your app. Linux builds are static with respect to OpenSSL and need glibc 2.35 or newer. The
Windows build links the C runtime statically (no VC++ redistributable needed).

## Notes

- Calls run on an internal large-stack thread and are thread-safe; panics come back as
  `error.code: "panic"`.
- Database calls (`Migrate*` with a `url`) block until the database answers: run them off the UI thread.
- PostgreSQL URLs are `postgres://...`; for SQLite the `url` is a database file path.
