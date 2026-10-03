using Certo.Models;

namespace Certo.Db.Cli;

/// <summary><c>schema check|ir|diff</c>: work on SDL files.</summary>
public static class SchemaCommands
{
    /// <summary>Compile a schema file; on errors report them and return null.</summary>
    public static SdlCompileResult? Load(string path)
    {
        var source = File.ReadAllText(path);
        var result = CertoSdl.Compile(source);
        Output.Diagnostics(path, result.Diagnostics);
        return result.Ok ? result : null;
    }

    public static int Check(string[] args)
    {
        var a = new Args(args, [], ["quiet"]);
        var path = a.One("a schema file: schema check <file.sdl>");
        if (Load(path) is not { Schema: { } schema }) return 1;
        if (a.Flag("quiet")) return 0;
        foreach (var e in schema.Enums) Console.WriteLine($"enum {e.Name} {{ {string.Join(", ", e.Variants)} }}");
        foreach (var t in schema.Tables)
        {
            Console.WriteLine($"table {t.Name}");
            foreach (var c in t.Columns) Console.WriteLine($"  {c}");
            foreach (var i in t.Indexes) Console.WriteLine($"  index {i.Name} ({string.Join(", ", i.Columns)})");
            foreach (var k in t.Constraints) Console.WriteLine($"  check {k.Name}");
        }
        Console.Error.WriteLine($"{path}: ok ({schema.Tables.Count} table(s), {schema.Enums.Count} enum(s))");
        return 0;
    }

    /// <summary>Write the schema IR as JSON (what <c>IR.json</c> holds and QL is checked against).</summary>
    public static int Ir(string[] args)
    {
        var a = new Args(args, ["out"], []);
        var path = a.One("a schema file: schema ir <file.sdl> [-o IR.json]");
        if (Load(path) is not { } result) return 1;
        var json = result.EnsureOk();
        if (a.Value("out") is { } o) File.WriteAllText(o, json);
        else Console.WriteLine(json);
        return 0;
    }

    /// <summary>What it takes to turn one schema into another: a summary, the plan, or the SQL.</summary>
    public static int Diff(string[] args)
    {
        var a = new Args(args, ["mdl", "dialect"], ["sql", "json"]);
        var (oldPath, newPath) = a.Two("two schema files: schema diff <old.sdl> <new.sdl>");
        var (old, @new) = (Load(oldPath), Load(newPath));
        if (old is null || @new is null) return 1;
        var (oldIr, newIr) = (old.EnsureOk(), @new.EnsureOk());

        MigrationPlanResult plan;
        if (a.Value("mdl") is { } mdlPath)
        {
            plan = CertoPlans.Plan(oldIr, newIr, File.ReadAllText(mdlPath));
            Output.Diagnostics(mdlPath, plan.Diagnostics);
        }
        else plan = CertoPlans.Diff(oldIr, newIr);
        if (!plan.Ok) return 1;

        if (a.Flag("json")) { Console.WriteLine(plan.PlanJson); return 0; }
        if (a.Flag("sql"))
        {
            var sql = CertoSql.Lower(plan.PlanJson, a.Dialect(), oldIr, newIr);
            if (!sql.Ok)
            {
                Console.Error.WriteLine($"error: {sql.Error?.Message}");
                return 1;
            }
            Console.WriteLine(sql.Script);
            return 0;
        }
        if (plan.Empty) { Console.Error.WriteLine("no changes"); return 0; }
        foreach (var step in plan.Summary) Console.WriteLine(step.Text);
        if (plan.Destructive) Console.Error.WriteLine("(contains destructive operations)");
        return 0;
    }
}
