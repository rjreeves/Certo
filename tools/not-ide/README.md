# not-ide

A small Windows GUI for developing Certo projects. It organises a project's
`.cto` code, runs the `certo` toolchain with one click, lists compiler
errors, and opens files (at the exact line) in VS Code for editing.

## Build and run

Requires the .NET 8 SDK (`dotnet --version`).

    build.cmd                # produces publish\NotIde.exe
    publish\NotIde.exe [path-to-certo-project]

or `dotnet run` while developing.

## What it does

- **Project tree** with three views: *Folders*, *By module* (groups `.cto`
  files by their `module` line) and *By type*. Markers: `▶` has `fn main`,
  `✓tests` has test blocks, `◀ entry` is the `[build] entry` from certo.toml.
  Live filter box; the tree refreshes when files are added or removed.
- **Target** = the `.cto` file you select (or the certo.toml entry).
- **Toolbar**: Check, Build, Run, Test, Format (check / in place), Lint,
  More (Bench, Docs, Emit C, Build DLL, Audit), Database (status, migrate,
  dry run, rollback, new migration), Pipeline (fmt --check, check, lint, test,
  build), Stop.
- **Options**: Release (`--release`), Verbose (`-v`), Keep going, Scope
  (*Target file* / *All files*), program args for Run.
- **Problems list** parses `error[E0100]: ...` / `--> file:line:col` blocks
  and `file.cto:line:col: error: ...` lines; double-click opens VS Code there.
  The output pane also opens `file:line` on double-click.
- **VS Code**: double-click a file in the tree, or right-click for more.

Shortcuts: F5 run, Ctrl+B build, F6 check, F8 test, Shift+F5 stop, Ctrl+O open.

## Finding tools

certo.exe is looked up as: the saved path, `<project>\dist\certo.exe`,
`<project>\target\release\certo.exe`, `target\debug`, then PATH. VS Code is
found in `%LOCALAPPDATA%\Programs\Microsoft VS Code`, Program Files, or PATH.
Override either from the *More* menu. Settings live in
`%APPDATA%\NotIde\settings.json`.

Build output for single files goes to `<project>\out` (change under More).
The manifest entry builds with plain `certo build` so certo.toml rules apply.
Database commands need `DATABASE_URL` in the environment.
