using Certo.Models;

namespace Certo.Db.Cli;

/// <summary>What a command reports: data on standard output, everything else on standard error.</summary>
public static class Output
{
    /// <summary><c>file:line:column: severity CODE: message</c>, the form editors and CI understand.</summary>
    public static void Diagnostics(string file, IEnumerable<CertoDiagnostic> diagnostics)
    {
        foreach (var d in diagnostics)
        {
            var where = d.Span is { } s ? $"{file}:{s.Line}:{s.Column}" : file;
            Console.Error.WriteLine($"{where}: {d.Severity.ToString().ToLowerInvariant()} {d.Code}: {d.Message}");
        }
    }

    /// <summary>Report a failed call: its diagnostics if it has them, otherwise its error.</summary>
    public static void Failure(string file, CertoException e)
    {
        var diagnostics = e.Diagnostics.Count > 0 ? e.Diagnostics : e.Error?.Diagnostics ?? new List<CertoDiagnostic>();
        if (diagnostics.Count > 0)
        {
            Diagnostics(e.Error?.File ?? file, diagnostics);
            return;
        }
        Console.Error.WriteLine($"error: {e.Message}");
        if (e.Error is { Operations.Count: > 0 } err)
        {
            foreach (var op in err.Operations!) Console.Error.WriteLine($"  {op}");
            Console.Error.WriteLine("  (review them, then pass --allow-destructive)");
        }
        if (e.Error is { Items.Count: > 0 } drift)
            foreach (var item in drift.Items!) Console.Error.WriteLine($"  {item}");
        if (e.Error is { Applied.Count: > 0 } done)
            Console.Error.WriteLine($"  already applied: {string.Join(", ", done.Applied!)}");
        if (e.Error?.Statement is { } sql) Console.Error.WriteLine($"  statement: {sql}");
    }

    public static string Describe(SchemaType t, bool nullable) => $"{t}{(nullable ? "?" : "")}";
}
