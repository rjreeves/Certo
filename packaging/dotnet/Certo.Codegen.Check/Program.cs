// Compiles the generated C# and runs it against a real database: Microsoft.Data.Sqlite always,
// and Npgsql when CERTO_TEST_PG_URL is set (postgres://user@host:port/db; its `public` schema is reset).

using Microsoft.Data.Sqlite;
using MySqlConnector;
using Npgsql;

int failures = 0;
void Report(bool ok, string what)
{
    Console.WriteLine($"{(ok ? "ok  " : "FAIL")} {what}");
    if (!ok) failures++;
}

var dir = Path.Combine(AppContext.BaseDirectory, "generated");

Console.WriteLine("-- SQLite (Microsoft.Data.Sqlite)");
await using (var sqlite = new SqliteConnection("Data Source=:memory:"))
{
    await sqlite.OpenAsync();
    using var cmd = sqlite.CreateCommand();
    cmd.CommandText = File.ReadAllText(Path.Combine(dir, "schema.sqlite.sql"));
    cmd.ExecuteNonQuery();
    try { await Check_Sqlite.Run(sqlite, Report); }
    catch (Exception e) { Report(false, "SQLite scenario threw: " + e); }
}

var url = Environment.GetEnvironmentVariable("CERTO_TEST_PG_URL");
if (!string.IsNullOrEmpty(url))
{
    Console.WriteLine("-- PostgreSQL (Npgsql)");
    var u = new Uri(url);
    var cs = new NpgsqlConnectionStringBuilder
    {
        Host = u.Host, Port = u.Port > 0 ? u.Port : 5432, Database = u.AbsolutePath.TrimStart('/'),
        Username = Uri.UnescapeDataString(u.UserInfo.Split(':')[0]),
    };
    if (u.UserInfo.Contains(':')) cs.Password = Uri.UnescapeDataString(u.UserInfo.Split(':', 2)[1]);
    await using var pg = new NpgsqlConnection(cs.ConnectionString);
    await pg.OpenAsync();
    using var cmd = pg.CreateCommand();
    cmd.CommandText = "DROP SCHEMA public CASCADE; CREATE SCHEMA public; " + File.ReadAllText(Path.Combine(dir, "schema.pg.sql"));
    cmd.ExecuteNonQuery();
    try { await Check_Pg.Run(pg, Report); }
    catch (Exception e) { Report(false, "PostgreSQL scenario threw: " + e); }
}
else
{
    Console.WriteLine("-- PostgreSQL skipped (CERTO_TEST_PG_URL not set)");
}

var mysqlUrl = Environment.GetEnvironmentVariable("CERTO_TEST_MYSQL_URL");
if (!string.IsNullOrEmpty(mysqlUrl))
{
    Console.WriteLine("-- MySQL (MySqlConnector)");
    var m = new Uri(mysqlUrl);
    var mb = new MySqlConnectionStringBuilder
    {
        Server = m.Host, Port = (uint)(m.Port > 0 ? m.Port : 3306), Database = "mysql", UserID = Uri.UnescapeDataString(m.UserInfo.Split(':')[0]),
        DateTimeKind = MySqlDateTimeKind.Utc, AllowUserVariables = true,
    };
    if (m.UserInfo.Contains(':')) mb.Password = Uri.UnescapeDataString(m.UserInfo.Split(':', 2)[1]);
    await using var my = new MySqlConnection(mb.ConnectionString);
    await my.OpenAsync();
    using var cmd = my.CreateCommand();
    cmd.CommandText = "DROP DATABASE IF EXISTS certo_codegen; CREATE DATABASE certo_codegen; USE certo_codegen; SET SESSION time_zone = '+00:00'; "
        + File.ReadAllText(Path.Combine(dir, "schema.mysql.sql"));
    cmd.ExecuteNonQuery();
    try { await Check_Mysql.Run(my, Report); }
    catch (Exception e) { Report(false, "MySQL scenario threw: " + e); }
}
else
{
    Console.WriteLine("-- MySQL skipped (CERTO_TEST_MYSQL_URL not set)");
}

Console.WriteLine(failures == 0 ? "\nall checks passed" : $"\n{failures} check(s) FAILED");
return failures == 0 ? 0 : 1;

// the scenarios are the same source compiled against each provider's generated code
static class Check_Sqlite { public static Task Run(System.Data.Common.DbConnection c, Action<bool, string> k) => global::Check.Scenario_Sqlite.Run(c, k); }
static class Check_Pg { public static Task Run(System.Data.Common.DbConnection c, Action<bool, string> k) => global::Check.Scenario_Pg.Run(c, k); }
static class Check_Mysql { public static Task Run(System.Data.Common.DbConnection c, Action<bool, string> k) => global::Check.Scenario_Mysql.Run(c, k); }
