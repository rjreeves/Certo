using Certo.Models;

namespace Certo.Db.Cli;

/// <summary><c>ql check|compile|codegen</c>: typed queries and host code against a schema.</summary>
public static class QlCommands
{
    private static readonly string[] Options = ["schema", "dialect", "namespace", "class", "out"];

    /// <summary>Compile the schema and the query file; on errors report them and return null.</summary>
    private static (QlCompileResult Result, string Ir)? Compile(Args a, string queriesPath)
    {
        var schemaPath = a.Value("schema") ?? throw new UsageException("--schema <schema.sdl> is required");
        if (SchemaCommands.Load(schemaPath) is not { } schema) return null;
        var ir = schema.EnsureOk();
        var result = CertoQl.Compile(ir, File.ReadAllText(queriesPath), a.Dialect());
        Output.Diagnostics(queriesPath, result.Diagnostics);
        if (!result.Ok)
        {
            if (result.Error is { } e) Console.Error.WriteLine($"error: {e.Message}");
            return null;
        }
        return (result, ir);
    }

    /// <summary>List every statement with its typed contract.</summary>
    public static int Check(string[] args)
    {
        var a = new Args(args, Options, []);
        var path = a.One("a query file: ql check <file.ql> --schema <schema.sdl>");
        if (Compile(a, path) is not var (result, _)) return 1;
        foreach (var s in result.EnsureOk())
        {
            var ps = string.Join(", ", s.Params.Select(p => $"{p.Name}: {Output.Describe(p.Type, p.Nullable)}"));
            Console.WriteLine($"{s.Kind.ToString().ToLowerInvariant()} {s.Name}({ps})");
            if (s.Columns.Count > 0)
                Console.WriteLine($"  -> {string.Join(", ", s.Columns.Select(c => $"{c.Name} {Output.Describe(c.Type, c.Nullable)}"))}");
            else if (s.IsMutation)
                Console.WriteLine("  -> row count");
        }
        Console.Error.WriteLine($"{path}: ok ({result.Statements!.Count} statement(s))");
        return 0;
    }

    /// <summary>Print the SQL of every statement with its placeholders.</summary>
    public static int CompileSql(string[] args)
    {
        var a = new Args(args, Options, []);
        var path = a.One("a query file: ql compile <file.ql> --schema <schema.sdl>");
        if (Compile(a, path) is not var (result, _)) return 1;
        var marker = a.Dialect() == SqlDialect.Sqlite ? "?" : "$";
        foreach (var s in result.EnsureOk())
        {
            var binds = string.Join(", ", s.ParamOrder.Select((p, i) => $"{marker}{i + 1}={p}"));
            Console.WriteLine($"-- {s.Name}{(binds.Length > 0 ? $"  ({binds})" : "")}");
            Console.WriteLine($"{s.Sql};\n");
        }
        return 0;
    }

    /// <summary>Generate the typed C# for the statements.</summary>
    public static int Codegen(string[] args)
    {
        var a = new Args(args, Options, []);
        var path = a.One("a query file: ql codegen <file.ql> --schema <schema.sdl>");
        if (Compile(a, path) is not var (result, ir)) return 1;
        var generated = CertoQl.GenerateCSharp(ir, File.ReadAllText(path), a.Dialect(), a.Value("namespace"), a.Value("class"));
        var code = generated.EnsureOk();
        if (a.Value("out") is { } o)
        {
            File.WriteAllText(o, code);
            Console.Error.WriteLine($"{path}: {result.Statements!.Count} statement(s) -> {o}");
        }
        else Console.Write(code);
        return 0;
    }
}
