namespace Certo.Db.Cli;

/// <summary>A command was used wrongly (exit code 2).</summary>
public sealed class UsageException(string message) : Exception(message);

/// <summary>
/// Arguments of one command: positionals, <c>--name value</c> options and <c>--flag</c> switches.
/// Anything not declared is a usage error, so a typo never silently does the wrong thing.
/// </summary>
public sealed class Args
{
    private readonly Dictionary<string, string> _values = new();
    private readonly HashSet<string> _flags = new();
    public List<string> Positional { get; } = new();

    public Args(IEnumerable<string> args, string[] valueOptions, string[] flags)
    {
        var tokens = args.ToList();
        for (int i = 0; i < tokens.Count; i++)
        {
            var t = tokens[i];
            if (t == "-o") t = "--out";
            if (!t.StartsWith("--", StringComparison.Ordinal)) { Positional.Add(t); continue; }
            var name = t[2..];
            if (valueOptions.Contains(name))
            {
                if (i + 1 >= tokens.Count) throw new UsageException($"--{name} needs a value");
                _values[name] = tokens[++i];
            }
            else if (flags.Contains(name)) _flags.Add(name);
            else throw new UsageException($"unknown option --{name}");
        }
    }

    public string? Value(string name) => _values.GetValueOrDefault(name);
    public bool Flag(string name) => _flags.Contains(name);

    /// <summary>The single positional argument.</summary>
    public string One(string what)
    {
        if (Positional.Count != 1) throw new UsageException($"expected {what}");
        return Positional[0];
    }

    public (string, string) Two(string what)
    {
        if (Positional.Count != 2) throw new UsageException($"expected {what}");
        return (Positional[0], Positional[1]);
    }

    public SqlDialect Dialect()
    {
        var d = Value("dialect") ?? "postgres";
        return d.ToLowerInvariant() switch
        {
            "postgres" or "postgresql" or "pg" => SqlDialect.Postgres,
            "sqlite" or "sqlite3" => SqlDialect.Sqlite,
            _ => throw new UsageException($"unknown dialect `{d}` (postgres, sqlite)"),
        };
    }
}
