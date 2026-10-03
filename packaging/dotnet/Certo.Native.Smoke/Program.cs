// Smoke test for the packed Certo.Native package: every entry point once, the SQLite
// runner end to end, and concurrent calls. Exits non-zero if any check fails.

using System.Reflection;
using System.Text.Json;
using Certo;
using Certo.Models;
using Microsoft.Data.Sqlite;

int failures = 0;
void Check(bool ok, string what)
{
    Console.WriteLine($"{(ok ? "ok  " : "FAIL")} {what}");
    if (!ok) failures++;
}
JsonElement J(string json) => JsonDocument.Parse(json).RootElement.Clone();

// ---- library identity -------------------------------------------------------------------
var packageVersion = typeof(CertoNative).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()!
    .InformationalVersion.Split('+')[0];
var nativeVersion = CertoNative.Version();
Check(nativeVersion == packageVersion, $"native version {nativeVersion} matches package version {packageVersion}");

// ---- SDL -------------------------------------------------------------------------------------
const string schema = """
    enum Status { new, paid }
    table customers { id: serial primary key  email: varchar(100) not null unique  name: text }
    table orders {
        id: serial primary key
        customer_id: int not null references customers
        status: Status not null default new
        total: decimal(10,2) not null
    }
    """;
var compiled = J(CertoNative.CompileSdl(schema));
Check(compiled.GetProperty("ok").GetBoolean(), "CompileSdl accepts a valid schema");
var ir = compiled.GetProperty("ir").GetRawText();
var bad = J(CertoNative.CompileSdl("table t { id: nope }"));
Check(!bad.GetProperty("ok").GetBoolean() && bad.GetProperty("diagnostics").GetArrayLength() > 0,
    "CompileSdl reports positioned diagnostics for a bad schema");

// ---- plan and lowering ------------------------------------------------------------------
var emptyIr = J(CertoNative.CompileSdl("")).GetProperty("ir").GetRawText();
var diff = J(CertoNative.DiffIr(emptyIr, ir));
Check(diff.GetProperty("ok").GetBoolean(), "DiffIr");
var plan = diff.GetProperty("plan").GetRawText();
var pg = J(CertoNative.LowerSql(plan, "postgres"));
Check(pg.GetProperty("script").GetString()!.Contains("CREATE TABLE \"customers\""), "LowerSql (postgres)");
var needs = J(CertoNative.LowerSql(plan, "sqlite"));
Check(needs.GetProperty("error").GetProperty("code").GetString() == "needs_schemas", "LowerSql (sqlite) asks for schemas");
var lite = J(CertoNative.LowerSqlWithSchemas(plan, "sqlite", emptyIr, ir));
Check(lite.GetProperty("ok").GetBoolean() && lite.GetProperty("script").GetString()!.Contains("AUTOINCREMENT"),
    "LowerSqlWithSchemas (sqlite)");
var migration = J(CertoNative.PlanMigration(emptyIr, ir, ""));
Check(migration.GetProperty("ok").GetBoolean(), "PlanMigration");

// ---- QL: queries and mutations, both dialects ---------------------------------------------------
const string ql = """
    query recent(min: decimal(10,2), since: timestamp null) {
        from orders o join customers c on o.customer_id == c.id
        where o.total >= :min select o.id, c.name as customer, o.total order by o.id limit 10
    }
    query busy() {
        from customers c
        where exists (from orders o where o.customer_id == c.id select 1)
        select c.id, (from orders o where o.customer_id == c.id select count(*)) as orders
    }
    insert add(e: varchar(100), n: text null) { into customers set email = :e, name = :n returning id }
    insert add_two(a: varchar(100), b: varchar(100)) { into customers (email, name) values (:a, "A"), (:b, "B") returning id }
    update pay(id: int) { orders o set status = "paid" where o.id == :id }
    delete purge() { from orders o all rows }
    """;
foreach (var dialect in new[] { "postgres", "sqlite" })
{
    var r = J(CertoNative.CompileQl(ir, ql, dialect));
    var kinds = r.GetProperty("statements").EnumerateArray().Select(s => s.GetProperty("kind").GetString()).ToArray();
    Check(r.GetProperty("ok").GetBoolean() && string.Join(",", kinds) == "query,query,insert,insert,update,delete",
        $"CompileQl ({dialect}): {string.Join(",", kinds)}");
    var q = r.GetProperty("queries")[0];
    // o.id is NOT NULL; c.name is a nullable column, and `min` is the only parameter used
    Check(!q.GetProperty("columns")[0].GetProperty("nullable").GetBoolean()
          && q.GetProperty("columns")[1].GetProperty("nullable").GetBoolean()
          && q.GetProperty("param_order").GetArrayLength() == 1, $"CompileQl ({dialect}) typed contract");
}
var generated = J(CertoNative.CodegenQl(ir, ql, "{\"language\":\"csharp\",\"dialect\":\"sqlite\",\"namespace\":\"Smoke.Db\"}"));
var code = generated.GetProperty("code").GetString() ?? "";
Check(generated.GetProperty("ok").GetBoolean() && code.Contains("namespace Smoke.Db;") && code.Contains("public static async Task<List<RecentRow>> RecentAsync"),
    "CodegenQl returns a C# source file");
var qerr = J(CertoNative.CompileQl(ir, "query q() { from orders o select o.ghost }"));
Check(!qerr.GetProperty("ok").GetBoolean() && qerr.GetProperty("diagnostics")[0].GetProperty("code").GetString() == "QL206",
    "CompileQl reports QL diagnostics");

// ---- the SQLite runner, end to end ---------------------------------------------------------
var dir = Path.Combine(Path.GetTempPath(), "certo-smoke-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(dir);
try
{
    var db = Path.Combine(dir, "app.db");
    Check(J(CertoNative.MigrateInit(dir, "{\"dialect\":\"sqlite\"}")).GetProperty("ok").GetBoolean(), "MigrateInit (sqlite)");
    File.WriteAllText(Path.Combine(dir, "schema.sdl"), schema);
    var created = J(CertoNative.MigrateNew(dir, "{\"name\":\"init\"}"));
    Check(created.GetProperty("ok").GetBoolean(), "MigrateNew");
    Check(J(CertoNative.MigrateList(dir)).GetProperty("migrations").GetArrayLength() == 1, "MigrateList");
    var url = JsonSerializer.Serialize(new { url = db });
    var dry = J(CertoNative.MigrateApply(dir, JsonSerializer.Serialize(new { url = db, dry_run = true })));
    Check(dry.GetProperty("ok").GetBoolean() && dry.GetProperty("scripts").GetArrayLength() == 1,
        "MigrateApply dry run returns the script");
    Check(J(CertoNative.MigrateStatus(dir, JsonSerializer.Serialize(new { url = db }))).GetProperty("pending").GetArrayLength() == 1,
        "...and applied nothing");
    var applied = J(CertoNative.MigrateApply(dir, url));
    Check(applied.GetProperty("ok").GetBoolean(), "MigrateApply");
    var status = J(CertoNative.MigrateStatus(dir, url));
    Check(status.GetProperty("applied").GetArrayLength() == 1 && status.GetProperty("pending").GetArrayLength() == 0,
        "MigrateStatus: 1 applied, 0 pending");
    Check(J(CertoNative.MigrateDrift(dir, url)).GetProperty("in_sync").GetBoolean(), "MigrateDrift: in sync");

    // a second migration that needs a table rebuild
    File.WriteAllText(Path.Combine(dir, "schema.sdl"), schema.Replace("name: text", "name: text not null default \"anon\""));
    var second = J(CertoNative.MigrateNew(dir, "{\"name\":\"name_required\",\"allow_destructive\":true}"));
    Check(second.GetProperty("ok").GetBoolean(), "MigrateNew (table rebuild)");
    Check(J(CertoNative.MigrateApply(dir, url)).GetProperty("ok").GetBoolean(), "MigrateApply (table rebuild)");
    Check(J(CertoNative.MigrateDrift(dir, url)).GetProperty("in_sync").GetBoolean(), "MigrateDrift after the rebuild");

    // run compiled QL through an ordinary driver, binding by param_order
    var stmts = J(CertoNative.CompileQl(ir, ql, "sqlite")).GetProperty("statements").EnumerateArray().ToArray();
    using (var conn = new SqliteConnection($"Data Source={db};Pooling=False"))
    {
        conn.Open();
        var add = stmts.First(s => s.GetProperty("name").GetString() == "add");
    var busy = stmts.First(s => s.GetProperty("name").GetString() == "busy");
        using var insert = conn.CreateCommand();
        insert.CommandText = add.GetProperty("sql").GetString();
        var values = new Dictionary<string, object?> { ["e"] = "a@x.com", ["n"] = "Ann" };
        int i = 1;
        foreach (var p in add.GetProperty("param_order").EnumerateArray())
            insert.Parameters.AddWithValue($"?{i++}", values[p.GetString()!] ?? DBNull.Value);
        var id = insert.ExecuteScalar();
        Check(Convert.ToInt64(id) == 1, "compiled INSERT ... RETURNING runs through Microsoft.Data.Sqlite");
        using var sel = conn.CreateCommand();
        sel.CommandText = busy.GetProperty("sql").GetString();
        using var reader = sel.ExecuteReader();
        Check(!reader.Read(), "compiled correlated subqueries run (no customer has orders yet)");
    }

    var missing = J(CertoNative.MigrateStatus(dir, "{}"));
    Check(missing.GetProperty("error").GetProperty("code").GetString() == "missing_url", "a missing url is an error, not a crash");
}
finally
{
    try { Directory.Delete(dir, true); } catch { /* best effort */ }
}

// ---- adopt an existing SQLite database --------------------------------------------------------
var adoptDir = Path.Combine(Path.GetTempPath(), "certo-smoke-adopt-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(adoptDir);
try
{
    // a database nobody manages yet, made the way an application would
    var legacyDb = Path.Combine(adoptDir, "legacy.db");
    using (var legacy = new SqliteConnection($"Data Source={legacyDb};Pooling=False"))
    {
        legacy.Open();
        using var cmd = legacy.CreateCommand();
        cmd.CommandText = """
            CREATE TABLE customers (id INTEGER PRIMARY KEY AUTOINCREMENT, email VARCHAR(100) NOT NULL UNIQUE, name TEXT);
            CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id),
                                 total NUMERIC(10,2) NOT NULL);
            """;
        cmd.ExecuteNonQuery();
    }

    var fresh = Path.Combine(adoptDir, "fresh");
    Directory.CreateDirectory(fresh);
    CertoNative.MigrateInit(fresh, "{\"dialect\":\"sqlite\"}");
    var adopted = J(CertoNative.MigrateAdopt(fresh, JsonSerializer.Serialize(new { url = legacyDb })));
    Check(adopted.GetProperty("ok").GetBoolean() && adopted.GetProperty("schema_sdl").GetString()!.Contains("table orders"),
        "MigrateAdopt (sqlite)");
    Check(J(CertoNative.MigrateDrift(fresh, JsonSerializer.Serialize(new { url = legacyDb }))).GetProperty("in_sync").GetBoolean(),
        "the adopted project is in sync");
}
finally
{
    try { Directory.Delete(adoptDir, true); } catch { /* best effort */ }
}

// ---- the typed API ---------------------------------------------------------------------------
{
    var typedSchema = CertoSdl.Compile("""
        enum Role { admin, user }
        table users { id: serial primary key  name: varchar(100) not null  role: Role  balance: decimal(10,2) }
        """);
    Check(typedSchema.Ok && typedSchema.Diagnostics.Count == 0, "CertoSdl.Compile");
    var users = typedSchema.Schema!.Table("users")!;
    Check(users.Columns.Count == 4 && users.Columns[0] is { Name: "id", PrimaryKey: true, Generated: Generation.Serial }
          && users.Columns[1] is { Name: "name", Nullable: false } && users.Columns[1].Type.ToString() == "varchar(100)"
          && users.Columns[2].Type is { IsEnum: true, Name: "Role" } && users.Columns[3].Nullable,
        "typed schema: tables, columns, types, nullability, generation");
    Check(typedSchema.Schema!.Enums is [{ Name: "Role", Variants: ["admin", "user"] }], "typed schema: enums");
    Check(users.Columns[0].ToString() == "id serial primary key" && users.Columns[1].ToString() == "name varchar(100) not null",
        "a column describes itself in SDL's words");
    var linked = CertoSdl.Compile("table a { id: serial primary key }\ntable b { id: serial primary key  a_id: int not null references a on delete cascade }");
    Check(linked.Schema!.Table("b")!.Columns[1].References is { Table: "a", Column: "id", OnDelete: ReferentialAction.Cascade }, "typed schema: references and actions");
    Check(!CertoSdl.Compile("table t { id: nope }").Ok && CertoSdl.Compile("table t { id: nope }").Schema is null, "no typed schema when there are errors");
    var typedBad = CertoSdl.Compile("table t { id: nope }");
    Check(!typedBad.Ok && typedBad.Diagnostics[0].Severity == DiagnosticSeverity.Error && typedBad.Diagnostics[0].Span is { Line: 1 },
        "typed diagnostics carry severity, code and a position");
    try { typedBad.EnsureOk(); Check(false, "EnsureOk throws on errors"); }
    catch (CertoException e) { Check(e.Diagnostics.Count > 0 && e.Message.Contains("schema compilation failed"), "EnsureOk throws a CertoException with the diagnostics"); }

    var typed = CertoQl.Compile(typedSchema, """
        query find(min: decimal(10,2), r: Role null, tag: varchar(20) null) {
            from users u where u.balance >= :min and (:r is null or u.role == :r) select u.id, u.name, u.role, u.balance
        }
        insert add(n: varchar(100)) { into users set name = :n returning id }
        update bump(id: int) { users u set balance = 0 where u.id == :id }
        """, SqlDialect.Sqlite);
    var statements = typed.EnsureOk();
    Check(statements.Select(x => x.Kind).SequenceEqual(new[] { StatementKind.Query, StatementKind.Insert, StatementKind.Update }),
        "typed statements come back with their kinds");
    var find = statements[0];
    Check(find.Name == "find" && find.Params.Count == 3 && find.ParamOrder.SequenceEqual(new[] { "min", "r" }),
        "typed parameters and placeholder order (the unused `tag` is declared but not bound)");
    Check(find.Params[0].Type.ToString() == "decimal(10,2)" && find.Params[0].Type is { Precision: 10, Scale: 2 }, "numeric parameter type: decimal(10,2)");
    Check(find.Params[1].Type is { IsEnum: true, Name: "Role" } && find.Params[1].Nullable, "enum parameter, nullable");
    Check(find.Params[2].Type is { Name: "varchar", Length: 20 }, "varchar(n) parameter has its length");
    Check(find.Columns.Select(c => c.Name).SequenceEqual(new[] { "id", "name", "role", "balance" })
          && !find.Columns[0].Nullable && find.Columns[2].Nullable && find.Columns[2].Type.IsEnum,
        "typed result columns with nullability");
    Check(find.Columns[0].Type.Name == "int" && find.Columns[1].Type.ToString() == "varchar(100)", "builtin column types");
    Check(!find.IsMutation && find.ReturnsRows && statements[1].IsMutation && statements[1].ReturnsRows && statements[2].IsMutation && !statements[2].ReturnsRows,
        "IsMutation / ReturnsRows");
    Check(find.Sql.Contains("CAST(?1 AS NUMERIC(10,2))") && find.Ir.ValueKind == System.Text.Json.JsonValueKind.Object, "the SQL and the raw IR are available");
    Check(typed.Queries!.Count == 1, "Queries still holds only the read queries");

    var qbad = CertoQl.Compile(typedSchema, "query q() { from users u select u.ghost }");
    Check(!qbad.Ok && qbad.Statements is null && qbad.Diagnostics[0].Code == "QL206", "typed QL diagnostics");
    var warned = CertoQl.Compile(typedSchema, "query q(unused: int) { from users u select u.id }");
    Check(warned.Ok && warned.Diagnostics[0].Severity == DiagnosticSeverity.Warning, "a warning accompanies a success");
    var failed = CertoQl.Compile("nope", "query q() { from users u select u.id }");
    Check(!failed.Ok && failed.Error?.Code == "invalid_ir", "a failed call carries a typed error");

    var gen = CertoQl.GenerateCSharp(typedSchema.EnsureOk(), "query by_id(id: int) { from users u where u.id == :id select u.id }",
        SqlDialect.Sqlite, "My.Db", "Q");
    Check(gen.Ok && gen.EnsureOk().Contains("namespace My.Db;") && gen.Code!.Contains("public static partial class Q"), "typed code generation");
    // round trip: the type converter writes what it reads
    var rt = System.Text.Json.JsonSerializer.Serialize(find.Params[0].Type);
    Check(System.Text.Json.JsonSerializer.Deserialize<SchemaType>(rt)!.ToString() == "decimal(10,2)", "SchemaType round-trips through JSON");
}

// ---- typed plans, SQL and the migration runner ---------------------------------------------------
{
    var v1 = CertoSdl.Compile("table users { id: serial primary key  email: text not null }").EnsureOk();
    var v2 = CertoSdl.Compile("table users { id: serial primary key  email: text not null  age: int }").EnsureOk();
    var v3 = CertoSdl.Compile("table users { id: serial primary key  email: text }").EnsureOk();

    var tplan = CertoPlans.Diff(v1, v2).EnsureOk();
    Check(!tplan.Empty && !tplan.Destructive && tplan.Summary.Count == 1 && tplan.Summary[0].Text.Contains("age"), "typed plan: summary of a column add");
    Check(CertoPlans.Diff(v1, v1).EnsureOk().Empty, "typed plan: identical schemas give an empty tplan");
    var drop = CertoPlans.Diff(v2, v1).EnsureOk();
    Check(drop.Destructive && drop.Summary.Any(x => x.Destructive), "typed plan: a dropped column is destructive");

    var pgSql = CertoSql.Lower(tplan.PlanJson, SqlDialect.Postgres).EnsureOk();
    Check(pgSql.Batches.Count == 1 && pgSql.Batches[0].Transactional && pgSql.Batches[0].Statements[0].Contains("ADD COLUMN \"age\""), "typed SQL batches (postgres)");
    Check(pgSql.Script!.StartsWith("BEGIN;"), "typed SQL script");
    var tneeds = CertoSql.Lower(tplan.PlanJson, SqlDialect.Sqlite);
    Check(!tneeds.Ok && tneeds.Error?.Code == "needs_schemas", "sqlite without schemas is a typed error");
    var liteSql = CertoSql.Lower(tplan.PlanJson, SqlDialect.Sqlite, v1, v2).EnsureOk();
    Check(liteSql.Batches.Count == 1 && liteSql.Batches[0].Statements[0].Contains("ADD COLUMN"), "typed SQL (sqlite, with schemas)");
    var rebuild = CertoSql.Lower(CertoPlans.Diff(v1, v3).EnsureOk().PlanJson, SqlDialect.Sqlite, v1, v3).EnsureOk();
    Check(rebuild.Batches.Count == 3 && !rebuild.Batches[0].Transactional && rebuild.Batches[1].Transactional,
        "a SQLite rebuild is three batches: pragma off, transaction, pragma on");

    var badMdl = CertoPlans.Plan(v1, v2, "rename nope.x -> y");
    Check(!badMdl.Ok && badMdl.Diagnostics.Count > 0, "MDL problems are typed diagnostics");
    try { badMdl.EnsureOk(); Check(false, "EnsureOk throws for bad MDL"); }
    catch (CertoException e) { Check(e.Diagnostics.Count > 0, "EnsureOk on a tplan carries the MDL diagnostics"); }
    Check(CertoPlans.Plan(v1, v2, "").Ok, "an empty MDL migration is a plain diff");
    Check(CertoPlans.Diff("nope", v1).Error?.Code == "invalid_ir", "a bad IR is a typed error");

    // ---- the runner, end to end on SQLite
    var proj = Path.Combine(Path.GetTempPath(), "certo-typed-" + Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(proj);
    try
    {
        var db = Path.Combine(proj, "app.db");
        var init = CertoMigrations.Init(proj, SqlDialect.Sqlite).EnsureOk();
        Check(init.Dialect == "sqlite" && init.Root.Length > 0, "typed Init");

        var none = CertoMigrations.New(proj, new NewMigrationOptions { Name = "nothing" });
        Check(!none.Ok && none.Error?.Code == "no_changes", "no changes is a typed error");

        File.WriteAllText(Path.Combine(proj, "schema.sdl"), "table users { id: serial primary key  email: text not null }");
        var created = CertoMigrations.New(proj, new NewMigrationOptions { Name = "init" }).EnsureOk();
        Check(created.Seq == 1 && created.Name == "init" && created.Summary.Count == 1 && !created.Destructive, "typed New: seq, name, summary");
        Check(CertoMigrations.List(proj).EnsureOk().Migrations is [{ Label: "0001_init", Statements: >= 1 }], "typed List");

        var dry = CertoMigrations.Apply(proj, new ApplyOptions { Url = db, DryRun = true }).EnsureOk();
        Check(dry.DryRun && dry.Scripts.Count == 1 && dry.Scripts[0].Label == "0001_init" && dry.Scripts[0].Sql.Contains("CREATE TABLE"), "typed dry-run Apply");
        Check(CertoMigrations.Status(proj, db).EnsureOk().Pending.Count == 1, "...applied nothing");
        var applied = CertoMigrations.Apply(proj, new ApplyOptions { Url = db }).EnsureOk();
        Check(applied.Migrations.SequenceEqual(new[] { "0001_init" }), "typed Apply lists the migrations it applied");
        var status = CertoMigrations.Status(proj, db).EnsureOk();
        Check(status.Applied is [{ Seq: 1, Name: "init" }] && status.Pending.Count == 0 && status.Applied[0].AppliedAt.Length > 0, "typed Status");
        var driftNone = CertoMigrations.Drift(proj, new DriftOptions { Url = db }).EnsureOk();
        Check(driftNone.InSync && driftNone.Items.Count == 0 && driftNone.ExpectedFrom.Contains("0001_init"), "typed Drift: in sync");

        // a destructive migration tneeds permission
        File.WriteAllText(Path.Combine(proj, "schema.sdl"), "table users { id: serial primary key }");
        var refused = CertoMigrations.New(proj, new NewMigrationOptions { Name = "drop_email" });
        Check(!refused.Ok && refused.Error?.Code == "destructive" && refused.Error.Operations is { Count: > 0 }, "a destructive change is refused with the operations listed");
        try { refused.EnsureOk(); Check(false, "EnsureOk throws"); }
        catch (CertoException e) { Check(e.Error?.Code == "destructive", "EnsureOk carries the typed error"); }
        CertoMigrations.New(proj, new NewMigrationOptions { Name = "drop_email", AllowDestructive = true }).EnsureOk();
        CertoMigrations.Apply(proj, new ApplyOptions { Url = db, CheckDrift = true }).EnsureOk();

        // drift: a hand-made change, reported with a repair script
        using (var c = new SqliteConnection($"Data Source={db};Pooling=False"))
        {
            c.Open();
            using var cmd = c.CreateCommand();
            cmd.CommandText = "ALTER TABLE users ADD COLUMN sneaky INTEGER";
            cmd.ExecuteNonQuery();
        }
        var drifted = CertoMigrations.Drift(proj, new DriftOptions { Url = db }).EnsureOk();
        Check(!drifted.InSync && drifted.Items.Any(i => i.Kind == DriftKind.Unexpected && i.Text.Contains("sneaky")) && drifted.RepairSql is not null,
            "typed Drift: an unexpected column, with a repair script");
        var blocked = CertoMigrations.Apply(proj, new ApplyOptions { Url = db, CheckDrift = true });
        Check(blocked.Error?.Code == "schema_drift" && blocked.Error.Items is { Count: > 0 }, "Apply with CheckDrift refuses and lists the drift");

        // a schema with errors comes back as a typed compile error
        File.WriteAllText(Path.Combine(proj, "schema.sdl"), "table users { id: nope }");
        var broken = CertoMigrations.New(proj, new NewMigrationOptions { Name = "broken" });
        Check(broken.Error is { Code: "compile", Diagnostics.Count: > 0 }, "a schema error in New is a typed compile error with diagnostics");
        try { broken.EnsureOk(); Check(false, "EnsureOk throws"); }
        catch (CertoException e) { Check(e.Diagnostics.Count > 0, "...and EnsureOk exposes the diagnostics"); }

        var noUrl = CertoMigrations.Status(proj, "");
        Check(noUrl.Error?.Code == "missing_url", "a missing url is a typed error");
    }
    finally { try { Directory.Delete(proj, true); } catch { } }

    // adopting a database nobody manages yet
    var adoptProj = Path.Combine(Path.GetTempPath(), "certo-typed-adopt-" + Guid.NewGuid().ToString("N"));
    Directory.CreateDirectory(adoptProj);
    try
    {
        var legacy = Path.Combine(adoptProj, "legacy.db");
        using (var c = new SqliteConnection($"Data Source={legacy};Pooling=False"))
        {
            c.Open();
            using var cmd = c.CreateCommand();
            cmd.CommandText = "CREATE TABLE t (id INTEGER PRIMARY KEY, v VARCHAR(10) NOT NULL); CREATE VIEW vw AS SELECT id FROM t;";
            cmd.ExecuteNonQuery();
        }
        var pdir = Path.Combine(adoptProj, "p");
        Directory.CreateDirectory(pdir);
        CertoMigrations.Init(pdir, SqlDialect.Sqlite).EnsureOk();
        var preview = CertoMigrations.Adopt(pdir, new AdoptOptions { Url = legacy, DryRun = true }).EnsureOk();
        Check(preview.DryRun && preview.Migration is null && preview.Adopted is { Tables: 1, Columns: 2 }, "typed Adopt dry run: counts");
        Check(preview.Omissions.Any(o => o.Contains("vw")), "typed Adopt: omissions are listed");
        var adopted = CertoMigrations.Adopt(pdir, new AdoptOptions { Url = legacy }).EnsureOk();
        Check(adopted.SchemaSdl.Contains("table t") && adopted.Migration is not null, "typed Adopt: schema and baseline migration");
        Check(CertoMigrations.Drift(pdir, new DriftOptions { Url = legacy }).EnsureOk().InSync, "the adopted project is in sync");
        var again = CertoMigrations.Adopt(pdir, new AdoptOptions { Url = legacy });
        Check(again.Error?.Code == "project", "adopting twice is a typed error");
    }
    finally { try { Directory.Delete(adoptProj, true); } catch { } }
}

// ---- concurrency ------------------------------------------------------------------------------
var results = Enumerable.Range(0, 32).AsParallel().WithDegreeOfParallelism(8)
    .Select(i => J(CertoNative.CompileSdl($"table t{i} {{ id: int primary key }}")).GetProperty("ok").GetBoolean())
    .ToArray();
Check(results.All(x => x), "32 concurrent CompileSdl calls");

Console.WriteLine(failures == 0 ? "\nall checks passed" : $"\n{failures} check(s) FAILED");
return failures == 0 ? 0 : 1;
