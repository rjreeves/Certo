# Certo developer guide: using Certo from your own code

The `certo` command line is built on a library you can use directly. This guide is for the developer who wants to:

1. **call the compiler from a program**, to compile schemas and queries, plan migrations or apply them (section 2);
2. **generate typed code** from QL and call your database through it (section 3);
3. **embed migrations** in an application's start-up (section 4);
4. understand **how Certo is built** well enough to extend or debug it (section 5).

The language itself is described in the [language manual](language-manual.md); running things from a terminal in the [CLI manual](https://github.com/rjreeves/certo-cli/blob/main/docs/cli-manual.md).

## 1. The layers

Certo is a stack of small libraries. Each layer depends only on the ones above it in this list:

| Layer | Crate | Does |
|-------|-------|------|
| SDL | `certo-sdl` | Parses a schema and produces the **schema IR**: a typed, JSON-serialisable description of tables, enums, indexes, views. |
| MDL | `certo-mdl` | Parses migration directives (renames, remaps, backfills, steps). |
| Planner / SQL | `certo-sql` | Diffs two schema IRs into a **plan**, and lowers a plan to SQL for a dialect (PostgreSQL, SQLite, MySQL). |
| QL | `certo-ql` | Parses and checks queries against a schema IR; lowers them to SQL; generates C# and Rust code. |
| Runner | `certo-runner` | Talks to databases: applies migrations, records them, detects drift, reads a schema back from a database, runs QL. |
| C ABI | `certo-capi` | A `cdylib` exposing all of the above as functions that take and return JSON text. |
| .NET binding | `Certo.Native` (NuGet) | The native library for five platforms plus a typed C# API over the C ABI. |
| CLI | `Certo.Cli` | The `certo` tool, written in C# on `Certo.Native`. |

Everything between the layers is **data**: the schema IR, the plan and the checked-statement IR are plain JSON. That is what makes the tools composable: the same IR drives migrations, drift checking and code generation.

**Which one should I use?**

- A .NET application, or a script in C#: `Certo.Native`.
- A Rust application: depend on the crates directly (`certo-sdl`, `certo-ql`, `certo-runner` ...), or run `certo ql codegen --lang rust` and keep only the generated file.
- Any other language that can load a C library: call the C ABI (`certo-capi`) with JSON.
- No code at all: the CLI.

## 2. Certo.Native: the typed C# API

### 2.1 Installing

`Certo.Native` is published to a private GitHub Packages feed (not nuget.org). GitHub requires a sign-in to read it: make a personal access token (classic) with `read:packages` and add the feed:

```bash
dotnet nuget add source "https://nuget.pkg.github.com/rjreeves/index.json" \
    --name certo --username <your GitHub user> --password <token> --store-password-in-clear-text
dotnet add package Certo.Native --version 0.22.0
```

In CI, use the workflow's own `GITHUB_TOKEN` (the job needs `packages: read`) as the password. The package carries the native library for `win-x64`, `linux-x64`, `linux-arm64`, `osx-x64` and `osx-arm64`; NuGet copies the right one next to your app. Linux builds need glibc 2.35 or newer.

### 2.2 Compile a schema

```csharp
using Certo;
using Certo.Models;

var schema = CertoSdl.Compile("""
    enum Status { open, done }
    table tasks {
        id: serial primary key
        title: text not null
        status: Status not null default open
    }
    """);

foreach (var d in schema.Diagnostics) Console.WriteLine(d);      // "error SDL100 at 1:19: ..."
foreach (var t in schema.Schema!.Tables) Console.WriteLine(t.Name);
```

`"ok": false` is a normal outcome: schema errors come back as positioned **diagnostics** (severity, code, message, a 1-based line and column) in `Diagnostics`, and nothing throws. Call `schema.EnsureOk()` to turn errors into a `CertoException` that carries them. Only an ABI mismatch between the binding and the native library throws by itself.

`CertoSdl.Import(...)` reads a schema back from a live database.

### 2.3 Compile queries

```csharp
var ql = CertoQl.Compile(schema, """
    query open_tasks() { from tasks t where t.status == "open" select t.id, t.title }
    insert add_task(title: text) { into tasks set title = :title returning id }
    """, SqlDialect.Sqlite);

foreach (var s in ql.EnsureOk())
{
    Console.WriteLine($"{s.Kind} {s.Name}({string.Join(", ", s.Params.Select(p => $"{p.Name}: {p.Type}{(p.Nullable ? "?" : "")}"))})");
    foreach (var c in s.Columns) Console.WriteLine($"  -> {c.Name}: {c.Type}{(c.Nullable ? "?" : "")}");
    // s.Sql         the SQL for the dialect
    // s.ParamOrder  which declared parameter is $1, $2, ... (or ?1, ?2 ...)
    // s.Ir          the full checked IR, as JSON
}
```

A `QlStatement` has everything a tool needs to run a statement without guessing: kind (`query`, `insert`, `update`, `delete`), name, parameters with their `SchemaType` and nullability, result columns with type and nullability, the SQL and the placeholder order. `SchemaType.ToString()` gives `varchar(100)`, `decimal(10,2)` or an enum's name.

### 2.4 Plans and SQL

```csharp
var plan = CertoPlans.Diff(oldIr, newIr).EnsureOk();      // plan.Summary, plan.Destructive, plan.PlanJson
var sql  = CertoSql.Lower(plan.PlanJson, SqlDialect.Sqlite, oldIr, newIr).EnsureOk();   // sql.Batches
```

`Diff` compares two schema IRs (optionally with MDL directives) and returns a plan: an ordered list of operations such as `create_table`, `add_column`, `rename_column`, `drop_index`. `plan.Destructive` lists the operations that can lose data. `Lower` turns a plan into SQL batches for a dialect; SQLite needs both schemas because it rebuilds tables for many alterations.

### 2.5 Migrations

```csharp
CertoMigrations.Init(dir, SqlDialect.Sqlite).EnsureOk();
// write dir/schema.sdl, then:
var created = CertoMigrations.New(dir, new NewMigrationOptions { Name = "init" }).EnsureOk();
var applied = CertoMigrations.Apply(dir, new ApplyOptions { Url = "app.db" }).EnsureOk();
var drift   = CertoMigrations.Drift(dir, new DriftOptions { Url = "app.db" }).EnsureOk();   // drift.InSync, drift.Items, drift.RepairSql
```

The same operations as the CLI: `Init`, `New`, `List`, `Status`, `Apply`, `Drift`, `Adopt`. A project directory has the layout described in section 5.3. Database URLs are `postgres://user:pass@host/db`, `mysql://user:pass@host/db`, or, for SQLite, a file path.

### 2.6 Errors

A call that fails without throwing carries a typed `Error`. `result.Error?.Code` is one of:

| Code | Meaning |
|------|---------|
| `destructive` | The plan can lose data; `Error.Operations` lists the operations. Pass `AllowDestructive` in `NewMigrationOptions` to accept. |
| `no_changes` | The schema equals the last migration's. |
| `compile` | The schema or MDL has errors; `Error.Diagnostics` holds them. |
| `database` | A statement failed: `Seq`, `Statement` and the migrations already `Applied`. |
| `schema_drift` | The database differs from the recorded schema; `Error.Items` lists it. |
| `connection` | The database could not be reached. |
| `needs_schemas` | `Lower` for SQLite was called without both schemas. |
| `invalid_ir`, `unknown_dialect`, `panic`, ... | Misuse, or an internal fault (panics come back as a result rather than crashing the host). |

`EnsureOk()` converts any of these into a `CertoException`.

### 2.7 Threading and blocking

Calls run on an internal large-stack thread and are thread-safe. Calls that take a database URL (`Apply`, `Drift`, `Adopt`, `Import`) block until the database answers: run them off a UI thread.

### 2.8 The raw API

Under the typed layer, `Certo.CertoNative` has one function per capability, each taking and returning JSON text (`CompileSdl`, `CompileQl`, `CodegenQl`, `Plan`, `Lower`, `Migrate*` ...). Use it when you want the JSON itself, or from a language without the typed wrapper. The models ignore unknown members, so a newer native library does not break an older host.

## 3. Generated, typed data access

QL describes statements; `certo ql codegen` turns them into code that runs them. This is the way to use QL from an application: nothing to parse or bind by hand, and the compiler guarantees that the SQL, the parameter types and the result types agree with the schema.

### 3.1 C#

```bash
certo ql codegen queries.ql --schema schema.sdl --dialect sqlite -o Queries.cs --namespace App.Db
```

Given

```ql
query open_tasks(min_id: int) { from tasks t where t.status == "open" and t.id >= :min_id select t.id, t.title, t.note order by t.id }
insert add_task(title: text) { into tasks set title = :title returning id }
```

you get a file with a record per result and an extension method per statement (plain ADO.NET, so it works with Npgsql, Microsoft.Data.Sqlite and MySqlConnector):

```csharp
public sealed record OpenTasksRow(int Id, string Title, string? Note);

public static async Task<List<OpenTasksRow>> OpenTasksAsync(
    this DbConnection connection, int minId,
    DbTransaction? transaction = null, CancellationToken cancellationToken = default)
```

and you call it:

```csharp
var open = await connection.OpenTasksAsync(minId: 1);                    // List<OpenTasksRow>
var id   = (await connection.AddTaskAsync("write docs")).Single().Id;     // insert ... returning id
var changed = await connection.SetStatusAsync(id, Status.Done);           // update: rows affected
```

How the contract maps:

- A result column becomes a record property, **nullable only where QL says it can be NULL** (`string? Note`).
- A nullable parameter becomes a nullable argument.
- An enum becomes a C# enum, bound and read as its database text.
- Each statement's SQL is also a constant (`OpenTasksSql`) if you want to log or reuse it.
- A mutation without `returning` returns the affected row count.
- Everything is `async`, takes an optional transaction and a cancellation token.

| SDL type | C# type |
|----------|---------|
| `text`, `varchar`, `char`, `json` | `string` |
| `smallint`, `int`, `bigint` | `short`, `int`, `long` |
| `decimal`, `numeric` | `decimal` |
| `real`, `float` | `float`, `double` |
| `bool` | `bool` |
| `uuid` | `Guid` |
| `timestamp` | `DateTimeOffset` |
| `timestamp_naive` | `DateTime` |
| `date` | `DateOnly` |
| `bytes` | `byte[]` |

Options: `--namespace` (default `Certo.Generated`), `--class` (the static class holding the methods, default `CertoQueries`), `-o` to write a file instead of standard output. Generate from the same schema file you migrate, so code and database cannot drift apart. In a .NET project, regenerate before building:

```xml
<Target Name="GenerateQueries" BeforeTargets="BeforeBuild">
  <Exec Command="certo ql codegen queries.ql --schema schema.sdl --dialect postgres -o Generated/Queries.cs" />
</Target>
```

From code: `CertoQl.GenerateCSharp(schema, qlSource, SqlDialect.Sqlite, "App.Db").EnsureOk()` returns the same text.

### 3.2 Rust

```bash
certo ql codegen queries.ql --schema schema.sdl --dialect postgres --lang rust -o queries.rs
```

The output has a struct per result and a function per statement (`snake_case` of its name); the first lines say which crates the file needs:

```rust
pub struct OpenTasksRow { pub id: i32, pub title: String, pub note: Option<String> }

pub fn open_tasks<C: postgres::GenericClient>(client: &mut C, min_id: i32)
    -> Result<Vec<OpenTasksRow>, postgres::Error>
```

| Dialect | Sync driver | `--async` driver |
|---------|-------------|------------------|
| PostgreSQL | `postgres` (any `GenericClient`: a `Client` or `Transaction`) | `tokio-postgres` |
| SQLite | `rusqlite` (a `Connection`; a `Transaction` derefs to it) | `tokio-rusqlite` |
| MySQL | `mysql` (anything `Queryable`) | `mysql_async` |

Notes:

- Nullable columns and parameters are `Option<T>`; text is taken as `&str`, bytes as `&[u8]`.
- Enums become Rust enums with `as_db` / `from_db`.
- `decimal` is `rust_decimal::Decimal`, `uuid` is `uuid::Uuid`, `timestamp` is `chrono::DateTime<Utc>`, `timestamp_naive` is `chrono::NaiveDateTime`, `date` is `chrono::NaiveDate`, and `json` is a small generated `Json(String)`.
- Under `--async` for SQLite the work runs on the connection's own thread, so borrowed arguments are copied first.
- Each statement's SQL is a constant, `OPEN_TASKS_SQL`.
- `--lang rust` is not the default: `--async` applies to Rust only (it is refused for C#, which is async already).

### 3.3 Dialect differences you will meet

The generated code is checked **against the dialect you generate for**: a statement MySQL cannot run in the same way (for example `returning`) fails code generation with a QL27x diagnostic instead of producing code that breaks at run time. See section 4 of the language manual.

SQLite stores `uuid`, `timestamp`, `timestamp_naive` and `date` as text. The generated code reads and writes them as the types in the tables above, but compare or order timestamps in the application (or on the same text form) rather than relying on SQLite to understand them.

## 4. Embedding migrations in an application

You can apply migrations from the application at start-up rather than from the command line.

```csharp
var result = CertoMigrations.Apply(migrationsDir, new ApplyOptions { Url = connectionString, CheckDrift = true });
if (!result.Ok)
{
    switch (result.Error?.Code)
    {
        case "schema_drift": /* someone changed the database by hand: report result.Error.Items */ break;
        case "database":     /* a statement failed: result.Error.Statement; Applied has the ones that succeeded */ break;
    }
    result.EnsureOk();      // throws a CertoException carrying the details
}
```

Guarantees worth building on:

- Each migration records itself in the `_certo_migrations` table with a hash of its SQL. `up.json` is checksummed, so a migration file changed after it was applied is detected.
- With `CheckDrift = true` (the CLI's `--check-drift`), `apply` **checks for drift first**: it will not run on a database whose schema differs from what the migrations recorded.
- PostgreSQL and SQLite run a migration in one transaction. **MySQL cannot** roll DDL back, so the runner applies each statement on its own and records progress in `_certo_progress`; if a step fails, fixing the cause and running `apply` again **resumes** from the failed statement.
- A per-database lock stops two processes applying at once (an advisory lock on PostgreSQL, `GET_LOCK` on MySQL).
- Every apply, adopt and QL view change is written to the **journal** (`_certo_log`): who, when, what, and the full detail. `certo log` reads it.
- QL views (`views.ql`) are dropped before and recreated after a migration run, in dependency order.

For a database that already exists: `CertoSdl.Import(url)` produces the SDL for it, `CertoMigrations.Adopt` records a baseline migration without running it, and `Drift` tells you whether schema and database still agree. See the CLI manual for the workflow.

## 5. How Certo is built

### 5.1 The IR

Compiling SDL gives a **schema IR**: tables with columns (name, type, nullability, default, key and foreign-key information), enums, composite types, sequences, indexes, constraints and views. Defaults, constraints and view conditions are stored as a typed expression tree (`ExprIR`), not as SQL text. That is why one schema can be lowered to three dialects and why a database's SQL can be **read back** into the same IR: the introspection code parses each database's expressions back into `ExprIR`, which is also what drift detection compares.

QL compiles to a **statement IR**: the resolved tables, the typed expression tree, the parameters with their types, the result columns with nullability. Lowering to SQL and code generation both start from that IR; neither re-parses text.

### 5.2 Why dialects agree

Certo does not translate "SQL" between databases. It compiles one typed IR to each dialect and, when the dialects genuinely disagree, **refuses** with a diagnostic rather than guess. Concretely:

- Every parameter is cast to its declared type, so SQLite's loose typing and MySQL's implicit conversions cannot change a result.
- MySQL tables are created with the binary collation, so text comparison is exact everywhere.
- `timestamp` is always UTC; MySQL stores it as `DATETIME(6)` and SQLite as text.
- Types a database lacks (UUID, timestamptz, enums, JSON) are kept in the schema and recorded in column comments or checks so they read back as the same SDL type.
- Features without an identical equivalent (`returning` and `groups` frames on MySQL, composite types outside PostgreSQL, ...) are refused with a numbered diagnostic.

### 5.3 A project on disk

`certo migrate init` creates:

```text
certo-db.toml            dialect, schema path, migrations directory (no credentials)
schema.sdl               the schema you edit
IR.json                  the schema as of the last generated migration
views.ql                 optional: QL views
migrations/
  0001_init/
    up.json              the plan
    up.sql               the SQL for the project's dialect
    plan.json            the operations, with the destructive ones marked
    ir.json              the schema after this migration
    migration.mdl        optional: the directives used
```

`up.sql` is plain text you can read and review; the recorded `ir.json` is what the next migration is diffed against and what drift is checked against.

### 5.4 Building from source

```bash
cargo test --workspace            # most tests; live-database tests read CERTO_TEST_PG_URL / CERTO_TEST_MYSQL_URL and skip without them
cargo clippy --workspace
```

The `certo-capi` crate builds the native library; `packaging/dotnet/` holds `Certo.Native` (the package), a smoke test project and a code-generation check that builds generated C# against Npgsql, Microsoft.Data.Sqlite and MySqlConnector. The `Certo.Native package` workflow builds the native library for every platform, tests it and publishes the package.

### 5.5 Where to look

| I want to know about... | Read |
|-------------------------|------|
| Exact grammar of SDL, MDL, QL | [`docs/database/ebnf.md`](../database/ebnf.md) |
| Migration layout and guarantees | `crates/runner/README.md` |
| The typed C# API | `packaging/dotnet/Certo.Native/README.md` |
| QL details and the code generators | `crates/ql/README.md`, `crates/ql/src/{csharp,rust}.rs` |
| Why something is refused | the diagnostic code in the [language manual](language-manual.md) |
