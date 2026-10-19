# not-ide — project context for Claude Code

A Windows WinForms GUI (C#, .NET 8) for developing **Certo** projects. Certo is
the user's own statically typed language: `.cto` files compile to C, then to a
native exe via clang/gcc/cc/cl. The GUI organises a project's code, runs the
`certo` CLI toolchain, lists compiler errors, and opens files in VS Code
(editing happens in VS Code, not in this app).

This folder lives inside the Certo repo at `tools/not-ide`. The Certo repo root
is two levels up (`..\..`). Certo docs: `..\..\docs\CLI-TOOLCHAIN.md`,
`HOW-TO-COMPILE.md`, `ERROR-REFERENCE.md`.

## Build / run

    dotnet build                 # compile-check (do this after every change)
    dotnet run                   # run the GUI (optional arg: project folder)
    build.cmd                    # publish to .\publish\NotIde.exe

Needs the .NET 8+ SDK on Windows. `RollForward=Major` lets it run on newer runtimes.

## Status — IMPORTANT

The first version was written without being able to compile the WinForms
code (the author's sandbox had no Windows UI libraries). `Toolchain.cs` and
`Settings.cs` were compiled and tested; **`MainForm.cs` has never been
compiled**. First task: run `dotnet build`, fix any compile errors, then run
the app and fix runtime problems (layout/docking, splitter sizes, etc.).

## Files

- `Program.cs`   – entry point (high-DPI, runs MainForm).
- `Settings.cs`  – JSON settings in `%APPDATA%\NotIde\settings.json`.
- `Toolchain.cs` – no UI code: `DiagnosticParser` (parses `error[E0100]: msg` +
  `  --> file:line:col` blocks and `file.cto:l:c: error: msg` lines),
  `ProjectInfo` (reads certo.toml `[project] name`, `[build] entry`),
  `Scanner` (per-file module/main/tests detection), `Locator` (finds certo.exe
  and VS Code), `Runner` (streams a child process, cancellable), `ArgSplitter`.
- `MainForm.cs`  – all UI: project tree (Folders / By module / By type views),
  command + project toolstrips, Problems list, output pane, status bar.

## Behaviour to preserve

- certo.exe lookup order: saved path, `<project>\dist\certo.exe`,
  `target\release`, `target\debug`, then PATH.
- Commands act on the *target* (selected `.cto` file, else certo.toml entry),
  or on all `.cto` files when Scope = "All files". `test` only gets files that
  contain `test "` blocks; `build` in All scope only files with `fn main`.
- The certo.toml entry builds with plain `certo build` (manifest rules apply);
  other files use `build <file> -o <project>\out\<stem>.exe`.
- Opening a file in VS Code uses `Code.exe <projectRoot> -g file:line:col`.
- Exit codes: 0 ok, 1 compile/test/lint failure, 2 bad args.
- Database commands need `DATABASE_URL`; the GUI only hints at it.

## Known gaps / ideas

- `certo lint` / `fmt --check` output formats unverified; parser may need
  tuning against real output (run them on `..\..\examples\*.cto`).
- No in-app editor by design. No `certo new` scaffolding or `certo repl` yet.
- Ideas: build history, per-project profiles, run `certo --watch`, recent
  projects menu, clickable lint codes linking to docs.

## Conventions

- Keep `Toolchain.cs` UI-free so it stays unit-testable.
- UI updates only on the UI thread (process output is marshalled with BeginInvoke).
- Don't edit files outside this folder without asking; the Certo compiler repo is the user's.
