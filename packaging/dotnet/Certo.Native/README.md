# Certo.Native

The Certo database compiler, in-process. Two layers over the same native library:

- **Typed API** (`CertoSdl`, `CertoQl`, `CertoPlans`, `CertoSql`, `CertoMigrations`; models in `Certo.Models`): results come back as C# objects.
- **Raw API** (`Certo.CertoNative`): every call takes and returns JSON text.

```csharp
using Certo;
using Certo.Models;

var schema = CertoSdl.Compile("table users { id: serial primary key  email: varchar(100) not null unique }");
foreach (var d in schema.Diagnostics) Console.WriteLine(d);              // "error SDL100 at 1:19: ..."

var qlResult = CertoQl.Compile(schema, """
    query by_email(e: varchar(100)) { from users u where u.email == :e select u.id, u.email }
    insert add(e: varchar(100)) { into users set email = :e returning id }
    """, SqlDialect.Sqlite);

foreach (var s in qlResult.EnsureOk())                                   // throws CertoException with the diagnostics on errors
{
    Console.WriteLine($"{s.Kind} {s.Name}({string.Join(", ", s.Params.Select(p => $"{p.Name}: {p.Type}{(p.Nullable ? "?" : "")}"))})");
    foreach (var c in s.Columns) Console.WriteLine($"  -> {c.Name}: {c.Type}{(c.Nullable ? "?" : "")}");
    // s.Sql, s.ParamOrder (which declared parameter is $1, $2, ...), s.Ir (the full checked IR)
}

// generated, typed host code for the same statements
string code = CertoQl.GenerateCSharp(schema.EnsureOk(), qlSource, SqlDialect.Sqlite, "App.Db").EnsureOk();

// plans and SQL
var plan = CertoPlans.Diff(oldIr, newIr).EnsureOk();                    // plan.Summary, plan.Destructive, plan.PlanJson
var sql  = CertoSql.Lower(plan.PlanJson, SqlDialect.Sqlite, oldIr, newIr).EnsureOk();   // sql.Batches (SQLite needs both schemas)

// migrations
CertoMigrations.Init(dir, SqlDialect.Sqlite).EnsureOk();
// write dir/schema.sdl, then:
var created = CertoMigrations.New(dir, new NewMigrationOptions { Name = "init" }).EnsureOk();
var applied = CertoMigrations.Apply(dir, new ApplyOptions { Url = "app.db" }).EnsureOk();   // applied.Migrations
var drift   = CertoMigrations.Drift(dir, new DriftOptions { Url = "app.db" }).EnsureOk();  // drift.InSync, drift.Items, drift.RepairSql
```

A call that fails without throwing carries a typed `Error`: `result.Error?.Code` is `destructive` (with
`Error.Operations`), `no_changes`, `compile` (with `Error.Diagnostics`), `database` (the failing `Seq`,
`Statement` and the migrations already `Applied`), `schema_drift` (with `Error.Items`), `connection`,
`needs_schemas`, ... `EnsureOk()` turns any of them into a `CertoException`.

The models are plain classes: `QlStatement` (kind, name, parameters, result columns, SQL, placeholder
order, raw IR), `QlParam` / `QlColumn` (name, `SchemaType`, nullable), `SchemaType` (kind, name, length /
precision / scale; `ToString()` gives `varchar(100)`, `decimal(10,2)`, `Role`), `CertoDiagnostic` (severity,
code, message, `SourceSpan` with 1-based line and column) and `CertoError` (a failed call: `invalid_ir`,
`unknown_dialect`, ...). Unknown JSON members are ignored, so a newer native library does not break an
older host. The raw `CertoNative` calls remain for hosts that want the JSON itself.

`"ok": false` is a normal outcome (schema errors come back as positioned `diagnostics`): the typed API
returns them in `Diagnostics` and throws only if you call `EnsureOk()`. Only an ABI mismatch between the
binding and the native library throws on its own.

## Installing from the private feed

The package is published to this repository's GitHub Packages feed, not to nuget.org. GitHub
requires a sign-in even to read from it: create a personal access token (classic) with the
`read:packages` scope, then add the feed to the consuming project's `nuget.config` (a sample is
in `packaging/dotnet/nuget.config.sample`) and keep the token out of the file:

```
dotnet nuget add source "https://nuget.pkg.github.com/rjreeves/index.json" \
    --name certo --username <your GitHub user> --password <token> --store-password-in-clear-text
dotnet add package Certo.Native --version 0.4.0
```

In CI, use the workflow's own `GITHUB_TOKEN` (give the job `packages: read`) as the password.

## Generated, typed access

`CertoNative.CodegenQl(ir, qlSource, optionsJson)` (or `certo ql codegen`) turns QL into one C# file:
a record per result and an extension method per statement, plain ADO.NET so it works with Npgsql and
Microsoft.Data.Sqlite.

```csharp
var orders = await connection.RecentOrdersAsync(minTotal: 20m, since: null);   // List<RecentOrdersRow>
var id = (await connection.AddCustomerAsync("Ann", null)).Single().Id;          // insert ... returning id
var changed = await connection.SetStatusAsync(id, Status.Paid);                  // update: rows affected
```

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
