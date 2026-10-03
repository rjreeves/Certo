using Certo.Models;

namespace Certo.Db.Cli;

/// <summary>
/// <c>migrate init|new|list|status|apply|drift|adopt</c>: the migration runner on a project directory
/// (default: the current one). The database is <c>--url</c> or <c>$DATABASE_URL</c>: a
/// <c>postgres://</c> URL, or a file path for a SQLite project.
/// </summary>
public static class MigrateCommands
{
    private static string Dir(Args a) => Path.GetFullPath(a.Value("dir") ?? ".");

    private static string Url(Args a)
    {
        var url = a.Value("url");
        if (string.IsNullOrEmpty(url)) url = Environment.GetEnvironmentVariable("DATABASE_URL");
        return !string.IsNullOrEmpty(url)
            ? url
            : throw new UsageException("no database: pass --url (a postgres:// URL, or a file path for a sqlite project) or set DATABASE_URL");
    }

    /// <summary>Run a call, reporting a failure the way a CLI should.</summary>
    private static int Run(string what, Func<int> body)
    {
        try { return body(); }
        catch (CertoException e)
        {
            Output.Failure(what, e);
            return 1;
        }
    }

    public static int Init(string[] args)
    {
        var a = new Args(args, ["dir", "dialect"], []);
        return Run("migrate init", () =>
        {
            var r = CertoMigrations.Init(Dir(a), a.Dialect()).EnsureOk();
            Console.WriteLine($"created {r.Dialect} project in {r.Root}");
            Console.Error.WriteLine("edit schema.sdl, then run: certo-db migrate new <name>");
            return 0;
        });
    }

    public static int New(string[] args)
    {
        var a = new Args(args, ["dir", "mdl"], ["allow-destructive"]);
        var name = a.One("a name: migrate new <name>");
        return Run("migrate new", () =>
        {
            var mdl = a.Value("mdl");
            var r = CertoMigrations.New(Dir(a), new NewMigrationOptions
            {
                Name = name,
                MdlSource = mdl is null ? null : File.ReadAllText(mdl),
                MdlLabel = mdl,
                AllowDestructive = a.Flag("allow-destructive"),
            }).EnsureOk();
            Console.WriteLine($"created {r.Label}");
            foreach (var line in r.Summary) Console.WriteLine($"  {line}");
            if (r.Destructive) Console.Error.WriteLine("  (contains destructive operations: review up.sql before applying)");
            if (!string.IsNullOrWhiteSpace(r.Warnings)) Console.Error.Write(r.Warnings);
            Console.Error.WriteLine($"review {Path.Combine(r.Dir, "up.sql")}, then run: certo-db migrate apply");
            return 0;
        });
    }

    public static int List(string[] args)
    {
        var a = new Args(args, ["dir"], []);
        return Run("migrate list", () =>
        {
            var r = CertoMigrations.List(Dir(a)).EnsureOk();
            if (r.Migrations.Count == 0) Console.Error.WriteLine("no migrations");
            foreach (var m in r.Migrations) Console.WriteLine($"{m.Label}  ({m.Statements} statement(s) in {m.Batches} batch(es))");
            return 0;
        });
    }

    public static int Status(string[] args)
    {
        var a = new Args(args, ["dir", "url"], []);
        return Run("migrate status", () =>
        {
            var r = CertoMigrations.Status(Dir(a), Url(a)).EnsureOk();
            foreach (var m in r.Applied) Console.WriteLine($"applied  {m.Label}  ({m.AppliedAt})");
            foreach (var m in r.Pending) Console.WriteLine($"pending  {m.Label}");
            Console.Error.WriteLine(r.Pending.Count == 0 ? $"up to date ({r.Applied.Count} applied)" : $"{r.Pending.Count} pending");
            return 0;
        });
    }

    public static int Apply(string[] args)
    {
        var a = new Args(args, ["dir", "url", "to"], ["dry-run", "check-drift"]);
        return Run("migrate apply", () =>
        {
            int? to = a.Value("to") is { } t ? int.Parse(t) : null;
            var r = CertoMigrations.Apply(Dir(a), new ApplyOptions
            {
                Url = Url(a), DryRun = a.Flag("dry-run"), To = to, CheckDrift = a.Flag("check-drift"),
            }).EnsureOk();
            if (r.DryRun)
            {
                foreach (var s in r.Scripts) Console.WriteLine($"-- {s.Label}\n{s.Sql}\n");
                if (r.Scripts.Count == 0) Console.Error.WriteLine("nothing pending");
                return 0;
            }
            foreach (var m in r.Migrations) Console.WriteLine($"applied {m}");
            if (r.Migrations.Count == 0) Console.Error.WriteLine("nothing pending");
            return 0;
        });
    }

    /// <summary>Exit code 1 when the database has drifted, like <c>diff</c>.</summary>
    public static int Drift(string[] args)
    {
        var a = new Args(args, ["dir", "url"], ["sql", "json"]);
        return Run("migrate drift", () =>
        {
            var r = CertoMigrations.Drift(Dir(a), new DriftOptions { Url = Url(a), RepairSql = a.Flag("sql") }).EnsureOk();
            Console.Error.WriteLine($"comparing the database with {r.ExpectedFrom}");
            foreach (var n in r.Notes) Console.Error.WriteLine($"note: {n}");
            if (r.InSync) { Console.WriteLine("no drift"); return 0; }
            foreach (var i in r.Items) Console.WriteLine(i);
            if (a.Flag("sql"))
                Console.WriteLine(r.RepairSql is { } s
                    ? $"\n-- script that would bring the database back to the migrations (review before running):\n{s}"
                    : $"\nno repair script: {r.RepairError}");
            return 1;
        });
    }

    public static int Adopt(string[] args)
    {
        var a = new Args(args, ["dir", "url"], ["dry-run", "force"]);
        return Run("migrate adopt", () =>
        {
            var r = CertoMigrations.Adopt(Dir(a), new AdoptOptions { Url = Url(a), DryRun = a.Flag("dry-run"), Force = a.Flag("force") }).EnsureOk();
            var n = r.Adopted;
            Console.Error.WriteLine($"{(r.DryRun ? "would adopt" : "adopted")}: {n.Tables} table(s), {n.Columns} column(s), {n.Enums} enum(s), {n.Indexes} index(es), {n.Constraints} constraint(s)");
            foreach (var o in r.Omissions) Console.Error.WriteLine($"left out: {o}");
            foreach (var d in r.KnownDrift) Console.Error.WriteLine($"remaining difference: {d}");
            if (r.DryRun) Console.WriteLine(r.SchemaSdl);
            else Console.Error.WriteLine($"wrote schema.sdl and recorded {r.Migration} as applied");
            return 0;
        });
    }
}
