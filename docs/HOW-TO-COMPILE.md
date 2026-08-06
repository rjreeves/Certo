# How to Compile a Certo Program

A focused, task-oriented walkthrough of turning a `.cto` file into a running
program. For the full command reference (test, fmt, lint, db, migrate, ...)
see [CLI-TOOLCHAIN.md](CLI-TOOLCHAIN.md); this doc only covers compiling.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Your first build](#2-your-first-build)
3. [What `certo build` actually does](#3-what-certo-build-actually-does)
4. [`certo build` vs `certo run`](#4-certo-build-vs-certo-run)
5. [Common options](#5-common-options)
6. [Compiling multiple files](#6-compiling-multiple-files)
7. [Compiling a library instead of an app](#7-compiling-a-library-instead-of-an-app)
8. [Building from `certo.toml`](#8-building-from-certotoml)
9. [Programs that use PostgreSQL](#9-programs-that-use-postgresql)
10. [Inspecting the generated C](#10-inspecting-the-generated-c)
11. [Troubleshooting](#11-troubleshooting)
12. [Quick reference](#12-quick-reference)

---

## 1. Prerequisites

Certo transpiles to C and then shells out to a real C compiler to produce a
native binary. You need one of these on `PATH`:

- `clang` (recommended)
- `gcc`
- `cc`
- `cl` (MSVC, Windows)

On Windows, if none of those are found, Certo also probes
`C:\Program Files\LLVM\bin\clang.exe` and the `(x86)` equivalent as a
fallback. If no compiler is found anywhere, `certo build` fails fast with:

```
error: no C compiler found (tried cc, gcc, clang)
       install gcc or clang, or use --emit-c to get the C source
```

You do **not** need PostgreSQL headers unless your program imports
`Stdlib.Db` — see [§9](#9-programs-that-use-postgresql).

---

## 2. Your first build

Create `hello.cto`:

```
module Hello

fn main(): Unit [io] = println("Hello, world!")
```

Compile it:

```bash
certo hello.cto -o hello
```

(`certo <file>` is shorthand for `certo build <file>`.) Run the result:

```bash
./hello
# Hello, world!
```

Two things every compilable program needs:

- A `module <Name>` declaration as the first line.
- A `fn main(): Unit [io] = ...` entry point. The `[io]` effect annotation is
  required because `println` performs I/O — Certo's effect checker rejects a
  `main` that calls an I/O function without declaring `[io]`. `pub` on `main`
  is optional; the compiler always finds it regardless of visibility.

A module with no `main` (e.g. a library) compiles fine but only if you pass
`--emit-dll` — see [§7](#7-compiling-a-library-instead-of-an-app).

---

## 3. What `certo build` actually does

`certo build src/main.cto -o dist/app` runs this pipeline:

1. **Parse** the `.cto` source into an AST.
2. **Resolve imports** — sibling `.cto` files referenced via `import` are
   located and merged in.
3. **Type-check** — Algorithm-W/Hindley-Milner inference, trait bound
   checking, effect checking. Any error here aborts before any C is emitted.
4. **Lower** to HIR then MIR (Certo's own intermediate representations).
5. **Emit C** — a single translation unit: runtime preamble, the relevant
   slice of stdlib's C implementation, then your compiled code.
6. **Invoke the C compiler** (`clang`/`gcc`/`cc`/`cl`) with `-O2` against
   that generated C file.
7. **Write the native executable** (or DLL, with `--emit-dll`) to the `-o`
   path.

If you only want step 5's output without steps 6–7, use `--emit-c`
(see [§10](#10-inspecting-the-generated-c)). If you only want steps 1–3 as a
fast correctness check, use `certo check` instead — it never invokes a C
compiler.

---

## 4. `certo build` vs `certo run`

```bash
certo build src/main.cto -o dist/app   # compile, leave a binary on disk
certo run   src/main.cto               # compile to a temp dir, execute, clean up
```

`certo run` is for iteration — compiles to a temporary file, runs it
immediately, and deletes the temporary binary when the process exits.
Arguments after `--` are passed through to the program as `argv`:

```bash
certo run src/main.cto -- --port 9090
# inside the program: arg(1) == "--port", arg(2) == "9090"
```

Build flags can appear before `--`:

```bash
certo run src/main.cto -v -- hello world
# -v is a build flag; "hello" and "world" are program args
```

---

## 5. Common options

| Flag | Effect |
|---|---|
| `-o <path>` | Output path. Default: `<stem>` (Unix) / `<stem>.exe` (Windows). |
| `--emit-c` | Stop after generating C; write `<stem>.c` instead of compiling it. |
| `--emit-dll` | Compile to a shared library (`.dll` / `.so` / `.dylib`) instead of an executable. |
| `-v`, `--verbose` | Print the exact C compiler invocation. |
| `-w`, `--watch` | Recompile automatically whenever the source file changes. |

`-v` is the fastest way to see exactly what's being linked and with which
flags — useful when a build fails for a reason that isn't a Certo type
error:

```bash
certo build src/main.cto -v
# clang C:\Users\...\certo_abc123.c -o main.exe -O2 -Wno-int-to-pointer-cast ...
```

---

## 6. Compiling multiple files

Two ways to combine files into one build. Either pass them all explicitly:

```bash
certo build src/main.cto src/orders.cto src/users.cto -o dist/app
```

...or write one entry point and let `import` pull the rest in automatically:

```
// src/main.cto
module Main
import MyApp.Orders   // resolved to src/MyApp/Orders.cto or src/Orders.cto

fn main(): Unit [io] = ...
```

```bash
certo build src/main.cto -o dist/app
```

All imported declarations are merged into one module before type-checking —
there's no separate-compilation/linking step at the Certo level; the C
compiler only ever sees one generated translation unit.

---

## 7. Compiling a library instead of an app

A file with no `main` function compiles as a shared library:

```bash
certo build src/lib.cto --emit-dll -o dist/mylib.dll
```

`pub` functions are exported — `__declspec(dllexport)` on Windows, default
visibility elsewhere. On Windows, an import library (`mylib.lib`) is
generated alongside the DLL automatically.

By default the exported C symbol is `certo_<snake_case_name>` (e.g.
`pub fn addNumbers` → `certo_add_numbers`). To pin a specific exported name
(for a stable C ABI, or to match what a calling language expects), annotate
the function:

```
@export("my_custom_add")
pub fn addNumbers(a: Int, b: Int): Int = a + b
```

This only changes the *exported* symbol — calls to `addNumbers(...)` from
other Certo code still go through the internal `certo_add_numbers` symbol
unaffected, and `certo-ffi`'s generated C header picks up the custom name.

---

## 8. Building from `certo.toml`

If a `certo.toml` exists in the working directory, bare `certo` (no file
argument) reads it for defaults instead of requiring an explicit path:

```toml
[project]
name    = "my-app"
version = "0.1.0"
edition = "2026"

[build]
type   = "app"           # "app" or "lib"
target = "native"
output = "dist/"         # created automatically if missing
entry  = "src/main.cto"  # required when type = "app"
```

```bash
certo          # equivalent to: certo build src/main.cto -o dist/my-app
```

Setting `type = "lib"` (and omitting `entry`, since a library has no `main`)
makes `certo build` pass `--emit-dll` automatically:

```toml
[build]
type   = "lib"
target = "native"
output = "dist/"
```

`certo new <name>` scaffolds a working `certo.toml` plus a `src/main.cto`
for you — the fastest way to get a valid starting layout.

---

## 9. Programs that use PostgreSQL

Importing `Stdlib.Db` (directly, or transitively) makes `certo build`
automatically link `libpq`. It resolves PostgreSQL's include/lib paths in
this order:

1. `PG_INCLUDE` / `PG_LIB` environment variables, if set — always win.
2. Windows: probe `C:\Program Files\PostgreSQL\<version>\`, newest first.
3. Unix: run `pg_config --includedir --libdir`.
4. Fall back to the system search path (works if `libpq-dev`/`libpq-devel`
   is installed).

**Windows**, if auto-detection picks the wrong installed version:

```powershell
$env:PG_INCLUDE = "C:\Program Files\PostgreSQL\16\include"
$env:PG_LIB     = "C:\Program Files\PostgreSQL\16\lib"
certo build src/main.cto -o dist/app.exe
```

**Linux**, install the dev headers first:

```bash
apt-get install libpq-dev      # Debian/Ubuntu
dnf install libpq-devel        # Fedora/RHEL
```

A program that never imports `Stdlib.Db` never touches any of this — the
`libpq` link step is skipped entirely.

---

## 10. Inspecting the generated C

```bash
certo build src/main.cto --emit-c
# writes main.c next to the source — no compiler invoked
```

Useful for:

- Debugging a codegen issue (comparing what you expected against what was
  actually emitted).
- Calling Certo-compiled code from C directly.
- Understanding a cryptic C-compiler error by reading the line it points at.

---

## 11. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `error: no C compiler found (tried cc, gcc, clang)` | Install `gcc` or `clang` and ensure it's on `PATH`, or use `--emit-c` to get C source without compiling it. |
| `error: module has no 'main' function` | You ran `certo build`/`run` on a file with no `fn main`. Either add one, or you meant `--emit-dll` to build a library. |
| Parse/type error before any C is generated | The pipeline stops at step 3 ([§3](#3-what-certo-build-actually-does)) — nothing gets compiled until the program is well-typed. Run `certo check src/main.cto` for the fastest feedback loop. |
| C compiler error referencing `certo_pq_*` / `libpq` symbols | PostgreSQL headers/libs not found — see [§9](#9-programs-that-use-postgresql). |
| C compiler error on a line you didn't write | Re-run with `--emit-c` and open the generated `.c` file at that line — it's almost always traceable back to one specific Certo expression. |
| Build silently uses a stale binary | `certo run` deletes its temp binary on exit but `certo build -o <path>` overwrites in place — make sure your shell/IDE isn't caching an old copy of `<path>`. |

---

## 12. Quick reference

```bash
certo <file.cto>                      # build (shorthand)
certo build <file.cto> -o <out>       # build, explicit output path
certo build <file.cto> --emit-c       # emit C only, don't invoke a C compiler
certo build <file.cto> --emit-dll     # build a shared library
certo build <file.cto> -v             # print the C compiler command line
certo build <file.cto> -w             # rebuild on every save
certo run   <file.cto> [-- args]      # build to a temp file, run it, clean up
certo check <file.cto>                # type-check only, no C compiler involved
certo                                 # build using certo.toml's [build] entry
```

For everything else the toolchain does — testing, formatting, linting,
benchmarking, the REPL, database migrations — see
[CLI-TOOLCHAIN.md](CLI-TOOLCHAIN.md).
