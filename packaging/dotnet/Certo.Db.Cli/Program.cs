using Certo;
using Certo.Db.Cli;
using Certo.Models;

const string Usage = """
    certo-db: a reference command-line host for the Certo database compiler (Certo.Native)

    Usage: certo-db <command> [options]

    Schemas (SDL)
      schema check <file.sdl>                       compile and list tables, columns and enums
      schema ir <file.sdl> [-o IR.json]             write the compiled schema as JSON
      schema diff <old.sdl> <new.sdl> [--mdl f.mdl] [--sql | --json] [--dialect postgres|sqlite]

    Queries (QL)
      ql check <file.ql> --schema <file.sdl>        each statement's typed parameters and result columns
      ql compile <file.ql> --schema <file.sdl>      the SQL, with placeholders
      ql codegen <file.ql> --schema <file.sdl> [--namespace N] [--class C] [-o Queries.cs]
        (all ql commands take --dialect postgres|sqlite, default postgres)

    Migrations (a project directory: --dir, default .)
      migrate init [--dialect postgres|sqlite]
      migrate new <name> [--mdl f.mdl] [--allow-destructive]
      migrate list
      migrate status | apply | drift | adopt   [--url <database>]  ($DATABASE_URL by default)
        apply:  [--dry-run] [--to N] [--check-drift]      drift: [--sql]      adopt: [--dry-run] [--force]

    --version                                       the native library's version
    Exit codes: 0 ok, 1 the source or database has problems (or drift), 2 the command was used wrongly.
    """;

try
{
    if (args.Length == 0 || args[0] is "-h" or "--help" or "help") { Console.WriteLine(Usage); return args.Length == 0 ? 2 : 0; }
    if (args[0] == "--version") { Console.WriteLine($"certo-db using Certo.Native {CertoNative.Version()}"); return 0; }

    var rest = args.Skip(2).ToArray();
    return (args[0], args.Length > 1 ? args[1] : "") switch
    {
        ("schema", "check") => SchemaCommands.Check(rest),
        ("schema", "ir") => SchemaCommands.Ir(rest),
        ("schema", "diff") => SchemaCommands.Diff(rest),
        ("ql", "check") => QlCommands.Check(rest),
        ("ql", "compile") => QlCommands.CompileSql(rest),
        ("ql", "codegen") => QlCommands.Codegen(rest),
        ("migrate", "init") => MigrateCommands.Init(rest),
        ("migrate", "new") => MigrateCommands.New(rest),
        ("migrate", "list") => MigrateCommands.List(rest),
        ("migrate", "status") => MigrateCommands.Status(rest),
        ("migrate", "apply") => MigrateCommands.Apply(rest),
        ("migrate", "drift") => MigrateCommands.Drift(rest),
        ("migrate", "adopt") => MigrateCommands.Adopt(rest),
        _ => throw new UsageException($"unknown command `{string.Join(' ', args.Take(2))}` (see certo-db --help)"),
    };
}
catch (UsageException e)
{
    Console.Error.WriteLine($"error: {e.Message}");
    return 2;
}
catch (CertoException e)
{
    Output.Failure("certo-db", e);
    return 1;
}
catch (FileNotFoundException e)
{
    Console.Error.WriteLine($"error: cannot read {e.FileName}");
    return 2;
}
catch (DirectoryNotFoundException e)
{
    Console.Error.WriteLine($"error: {e.Message}");
    return 2;
}
