# certo-db

A reference command-line host for the Certo database compiler, built only on the typed `Certo.Native` API.
It shows how a real CLI uses the library and is the test bed for finding what the API is missing; it is
small enough to copy from.

```
certo-db schema check schema.sdl                     # tables, columns, enums (typed schema model)
certo-db schema diff old.sdl new.sdl --sql --dialect sqlite
certo-db ql check queries.ql --schema schema.sdl     # each statement's typed parameters and result columns
certo-db ql codegen queries.ql --schema schema.sdl --dialect sqlite --namespace App.Db -o Queries.cs
certo-db migrate init --dialect sqlite
certo-db migrate new add_orders
certo-db migrate apply --url app.db                  # or $DATABASE_URL; a postgres:// URL, or a file for SQLite
certo-db migrate drift --url app.db --sql            # exit code 1 when the database has drifted
```

`certo-db --help` lists everything. Diagnostics go to standard error as `file:line:column: error CODE: message`
(what editors and CI understand), data to standard output. Exit codes: 0 ok, 1 problems in the source or
database (or drift), 2 the command was used wrongly.

## Build and test

```powershell
pwsh packaging/dotnet/stage-native.ps1      # build and stage the native library for this machine
dotnet build packaging/dotnet/Certo.Db.Cli -c Release
pwsh packaging/dotnet/cli-check.ps1         # drives every command on SQLite and checks output and exit codes
```

It references the `Certo.Native` project rather than the package, so it always exercises the current code;
`cli-check.ps1` stages the native library itself if it is missing.

## What building it taught the API

- The typed API had no model of the compiled schema, so listing tables and columns meant digging in JSON.
  `SdlCompileResult.Schema` (`SchemaIr`: tables, columns with types and nullability, references, indexes,
  enums, sequences, composite types) came from this.
- Migration results gained a `Label` (`0003_add_slug`) instead of every caller formatting `Seq` and `Name`.
