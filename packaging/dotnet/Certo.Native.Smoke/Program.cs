// Smoke test for the packed Certo.Native package: every entry point once, the SQLite
// runner end to end, and concurrent calls. Exits non-zero if any check fails.

using System.Reflection;
using System.Text.Json;
using Certo;
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

// ---- concurrency ------------------------------------------------------------------------------
var results = Enumerable.Range(0, 32).AsParallel().WithDegreeOfParallelism(8)
    .Select(i => J(CertoNative.CompileSdl($"table t{i} {{ id: int primary key }}")).GetProperty("ok").GetBoolean())
    .ToArray();
Check(results.All(x => x), "32 concurrent CompileSdl calls");

Console.WriteLine(failures == 0 ? "\nall checks passed" : $"\n{failures} check(s) FAILED");
return failures == 0 ? 0 : 1;
