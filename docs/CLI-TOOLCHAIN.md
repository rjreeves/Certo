# Certo CLI & Toolchain Guide

Everything you need to build, test, format, document, and deploy Certo programs
from the command line.

---

## Table of Contents

1. [Quick overview](#1-quick-overview)
2. [certo new — scaffolding a project](#2-certo-new--scaffolding-a-project)
3. [certo build — compiling](#3-certo-build--compiling)
4. [certo run — compile and execute](#4-certo-run--compile-and-execute)
5. [certo check — type-check only](#5-certo-check--type-check-only)
6. [certo fmt — formatter](#6-certo-fmt--formatter)
7. [certo lint — static analysis](#7-certo-lint--static-analysis)
8. [certo test — test runner](#8-certo-test--test-runner)
9. [certo bench — benchmarking](#9-certo-bench--benchmarking)
10. [certo doc — documentation generator](#10-certo-doc--documentation-generator)
11. [certo repl — interactive REPL](#11-certo-repl--interactive-repl)
12. [certo generate — code generation](#12-certo-generate--code-generation)
13. [certo db — database tools](#13-certo-db--database-tools)
14. [certo migrate — migrations](#14-certo-migrate--migrations)
15. [certo.toml — project manifest](#15-certotoml--project-manifest)
16. [C compiler discovery and PostgreSQL linking](#16-c-compiler-discovery-and-postgresql-linking)
17. [Environment variables](#17-environment-variables)
18. [Exit codes](#18-exit-codes)
19. [CI recipe](#19-ci-recipe)

---

## 1. Quick overview

```
certo --version                        print version
certo <file.cto>                       compile (shorthand for build)
certo build <file.cto> [options]       compile
certo run   <file.cto> [-- args]       compile then run
certo check <file.cto>                 type-check without compiling
certo fmt   <file.cto>...              format in place
certo lint  <file.cto>...              static analysis
certo test  <file.cto>...              run test blocks
certo bench <file.cto>                 run bench_ functions
certo new   <project-name>             scaffold a project
certo doc   <file.cto>                 generate HTML docs
certo repl                             interactive REPL
certo generate validators ...          generate Certo from YAML
certo db    <subcommand>               database tools
certo migrate <subcommand>             migration alias
```

Running `certo` with no arguments in a directory that contains `certo.toml`
is equivalent to `certo build` using the `entry` declared in that file.

---

## 2. certo new — scaffolding a project

```sh
certo new <project-name> [--template <template>]
```

Creates a new directory `<project-name>/` with a complete project skeleton.

### Templates

| Template | What it creates |
|---|---|
| `default` | Hello-world entry point (default when `--template` is omitted) |
| `api` | HTTP JSON API with `Stdlib.Http` and a `/health` route |
| `lib` | Library with public exports and `--emit-dll`; no `main` |
| `cli` | CLI tool with argument parsing via `arg(i)` |

### Example

```sh
certo new my-api --template api
cd my-api
certo run src/main.cto
```

### Generated layout

```
my-api/
├── certo.toml          project manifest
├── .env.example        DATABASE_URL and PORT template
├── .gitignore
├── README.md
├── src/
│   └── main.cto        entry point
├── db/
│   └── migrations/
├── tests/
│   ├── unit/
│   └── integration/
└── dist/               compiled outputs (gitignored)
```

Module name is derived from the project name by converting kebab/snake-case
to camelCase (`my-api` → `myApi`).

---

## 3. certo build — compiling

```sh
certo build <file.cto> [options]
certo <file.cto> [options]          # shorthand
certo                               # reads entry from certo.toml
```

### What happens

1. Parse the `.cto` source file.
2. Resolve local `import` statements by looking for sibling `.cto` files.
3. Type-check (parse errors and type errors abort here).
4. Emit C: preamble → runtime header → stdlib C → user code.
5. Invoke a C compiler (`clang`, `gcc`, or `cc`) with `-O2`.
6. Write the native executable (or DLL).

### Options

| Flag | Description |
|---|---|
| `-o <path>` | Output path. Default: `<stem>` on Unix, `<stem>.exe` on Windows. |
| `--emit-c` | Stop after generating C; write `<stem>.c` instead of compiling it. |
| `--emit-dll` | Compile to a shared library (`.dll` / `.so` / `.dylib`). |
| `-v`, `--verbose` | Print the full C compiler command line. |
| `-w`, `--watch` | Watch the source file and rebuild whenever it changes. |
| `--help`, `-h` | Show usage. |

### Compiling to a library

```sh
certo build src/lib.cto --emit-dll -o dist/mylib.dll
```

Public (`pub`) functions are exported with `__declspec(dllexport)` on Windows
and default visibility on Unix. On Windows an import library `mylib.lib` is
also generated alongside the DLL.

### Inspecting the generated C

```sh
certo build src/main.cto --emit-c
# writes main.c — open it in your editor
```

Useful for debugging codegen issues or calling Certo from C.

### Multi-file builds

Pass multiple `.cto` files to `certo build`. Their declarations are merged
into one module before type-checking:

```sh
certo build src/main.cto src/orders.cto src/users.cto -o dist/myapp
```

Alternatively, use `import` and let the compiler resolve files automatically:

```
// src/main.cto
import MyApp.Orders   // resolved to src/MyApp/Orders.cto or src/Orders.cto
```

### Watch mode

```sh
certo build src/main.cto --watch
```

Polls the source file and rebuilds on change. Useful during development.
Press `Ctrl+C` to stop.

---

## 4. certo run — compile and execute

```sh
certo run <file.cto> [build-options] [-- prog-args]
```

Compiles to a temporary directory and immediately executes the result.
Everything after `--` is passed to the compiled program as `argv`.

```sh
certo run src/main.cto                        # no program args
certo run src/main.cto -- --port 9090         # arg(1)="--port", arg(2)="9090"
certo run src/main.cto -v -- hello world      # -v is a build flag, rest are prog args
```

The temporary executable is deleted when the process exits.

---

## 5. certo check — type-check only

```sh
certo check <file.cto> [-v]
```

Parses and type-checks the file without invoking the C compiler. Useful in
editors and CI pipelines when you want fast feedback without a full build.

```sh
certo check src/main.cto     # exits 0 on success, 1 on error
certo check src/main.cto -v  # also prints "checking <file>..."
```

Exit codes: `0` = clean, `1` = parse or type errors, `2` = bad arguments.

---

## 6. certo fmt — formatter

```sh
certo fmt <file.cto>...
certo fmt --check <file.cto>...
```

Formats Certo source files in place using the canonical style. The formatter
re-parses the file and rebuilds it from the AST, so it is idempotent.

| Flag | Description |
|---|---|
| `--check` | Exits `1` if any file would be reformatted (no writes). |

### Usage in CI

```sh
certo fmt --check src/*.cto    # fail the build if files are not formatted
```

Files that have parse errors are skipped with a warning; the formatter does
not modify them.

---

## 7. certo lint — static analysis

```sh
certo lint <file.cto>...
```

Runs a dataflow pass over the HIR and reports warnings. Exits `1` if any
warnings are found.

### Lint codes

| Code | Description |
|---|---|
| `L001` | Unused parameter |
| `L002` | Unused variable — `val`/`var` declared but never read |
| `L003` | Assigned but never read — value written then immediately overwritten |
| `L004` | Unreachable statement — code after `panic`, `todo`, or `unreachable` |
| `L005` | Guard condition is a literal `true` or `false` |

### Suppressing warnings

Prefix the name with `_` to suppress `L001` and `L002` for that binding:

```
fn process(_unused: Int, result: Int): Int = result
val _temp = expensiveComputation()
```

---

## 8. certo test — test runner

```sh
certo test <file.cto>...
```

Compiles and runs all `test` blocks in the given files. Each test is
independently compiled and executed.

```
test "addition works" {
    assert(1 + 1 == 2, "expected 2")
}

test "list fold" {
    val total = List.fold([1, 2, 3], 0, (acc, x) => acc + x)
    assert(total == 6, "expected 6")
}
```

### Options

| Flag | Description |
|---|---|
| `--timeout=N` | Per-test timeout in milliseconds (default: 5000). |
| `--no-color` | Disable ANSI colour in output. |

### Output

```
running 3 tests in src/math_tests.cto
  test "addition works"    ok      1ms
  test "list fold"         ok      0ms
  test "divide by zero"    FAILED  (assertion failed: expected Err)

1 test failed.
```

Exit code is `0` when all tests pass, `1` if any fail. If a file has no
`test` blocks the runner prints a notice and moves to the next file.

---

## 9. certo bench — benchmarking

```sh
certo bench <file.cto> [--iterations=N]
```

Compiles and runs all zero-argument functions whose names start with `bench_`.
Each benchmark is run for a warmup pass (10 % of N) then for N iterations,
and the average nanoseconds per iteration is reported.

```
fn bench_sort(): Int = {
    val xs = [5, 3, 1, 4, 2]
    val sorted = List.sort(xs, (a, b) => a - b)
    List.len(sorted)
}
```

```sh
certo bench src/benchmarks.cto --iterations=10000
```

Output:

```
running 1 benchmarks (10000 iterations each) …

bench  bench_sort                                      124 ns/iter
```

The C compiler flag is `-O1` (not `-O2`) to prevent the optimizer from
eliminating benchmark loops entirely.

---

## 10. certo doc — documentation generator

```sh
certo doc <file.cto> [-o <dir>]
```

Generates HTML documentation from `///` doc comments. Output defaults to
`docs/` next to the source file.

```
/// Compute the hypotenuse of a right triangle.
/// Uses the Pythagorean theorem: sqrt(a² + b²).
pub fn hypotenuse(a: Float, b: Float): Float = sqrt(a * a + b * b)
```

```sh
certo doc src/main.cto            # writes docs/index.html
certo doc src/main.cto -o site/   # writes site/index.html
```

Only `pub` declarations are documented. Private functions appear in a
collapsed "internals" section. The generated HTML is self-contained with
embedded CSS — no external assets needed.

---

## 11. certo repl — interactive REPL

```sh
certo repl
certo           # same thing when no certo.toml is present
```

Opens an interactive read-eval-print loop. Each expression is compiled to C,
compiled with the system C compiler, and executed immediately. Stdlib modules
are available without explicit imports.

```
certo> 1 + 1
2
certo> val x = [1, 2, 3]
certo> List.fold(x, 0, (acc, n) => acc + n)
6
certo> :quit
```

The REPL is useful for quickly testing expressions and exploring the stdlib.
It is not suitable for multi-line function definitions; use a file and
`certo run` for that.

---

## 12. certo generate — code generation

```sh
certo generate validators --constraints <file.yaml> \
                           --temporals  <file.yaml> \
                           --entities   <file.yaml> \
                           [-o <output-dir>] \
                           [--emit-sql <sql-dir>]
```

Reads YAML rule definitions and emits:

- **Certo source** (always):
  - `constraints.cto` — prepared-item constraints
  - `temporals.cto` — time-based rules
  - `<entity>-validators.cto` — one validator per entity/trigger pair

- **PL/pgSQL** (with `--emit-sql <dir>`):
  - `004_rule_types.sql` — shared DB infrastructure
  - `005_validators/<entity>_<trigger>.sql` — one file per rule
  - `006_rule_triggers.sql` — trigger wiring
  - `007_api_helpers.sql` — preflight JSONB wrappers

This command is for projects that maintain validator rules in YAML (useful
when non-programmers need to edit business rules). Hand-written validators
using the `validator` keyword are the more common approach; see
[HOW-TO-PROGRAM.md](HOW-TO-PROGRAM.md#18-validators).

---

## 13. certo db — database tools

`certo db` is the umbrella command for all database operations. It requires
`DATABASE_URL` to be set (see [§17](#17-environment-variables)).

```sh
certo db <subcommand> [options]
```

### Subcommands

| Subcommand | Description |
|---|---|
| `migrate [--dry-run]` | Apply all pending migrations |
| `rollback [N]` | Roll back N migrations (default 1) |
| `status` | Show applied vs pending migrations |
| `create <name>` | Scaffold a new migration file |
| `pull [-o <file>]` | Introspect live DB → `db/schema.cto` |
| `diff <file.cto>` | Compare type declarations to live DB |

### certo db status

```sh
certo db status
```

```
Migration                                Applied At
------------------------------------------------------------
001 create users table                   2025-06-01 09:12:34
002 add users.role column                2025-06-10 14:22:01
003 create orders table                  (pending)
```

### certo db migrate

```sh
certo db migrate            # apply all pending migrations
certo db migrate --dry-run  # print the SQL without executing it
```

### certo db rollback

```sh
certo db rollback           # roll back the last migration
certo db rollback 3         # roll back the last 3 migrations
certo db rollback --dry-run # print the rollback SQL without executing
```

### certo db create

```sh
certo db create add_products_table
```

Writes `migrations/add_products_table.cto` with an empty template:

```
migration "add_products_table" {
    up {
        // TODO: add operations
    }
    down {
        // TODO: add rollback operations
    }
}
```

### certo db pull

Introspects the live PostgreSQL database and writes a `db/schema.cto` file
containing Certo `type` declarations, `fromRow` mapper functions, and
generated CRUD helpers.

```sh
certo db pull                         # writes db/schema.cto
certo db pull -o src/schema.cto       # custom output path
certo db pull --schema myschema       # non-default PostgreSQL schema
certo db pull -o -                    # print to stdout
```

The generated file looks like:

```
module DbSchema

// Generated by `certo db pull` — do not edit manually.

type User {
    id:        Int   // PK
    name:      Text
    email:     Text
    active:    Bool
    createdAt: DateTime
}

fn userFromRow(row: List<Text?>): User = ...
impl DbRow for User {}

fn userFindAll(conn: Int): List<User> [io] = ...
fn userFindById(conn: Int, id: Int): User? [io] = ...
fn userDeleteById(conn: Int, id: Int): Int [io] = ...
fn userInsert(conn: Int, record: User): Int [io] = ...
```

Re-run `db pull` whenever the live schema changes and commit the result.

Requires `psql` on `PATH`.

### certo db diff

Compares the `type` declarations in a `.cto` file against the live database
and reports any schema drift.

```sh
certo db diff src/domain.cto
certo db diff src/domain.cto --schema myschema
```

Output when in sync:

```
schema in sync — 4 table(s) match the live database
```

Output when drift is detected (exits `1`):

```
schema drift detected:

  TABLE  users
    MISSING COLUMN  users.last_login_at: DateTime?
    NULLABLE DRIFT  users.role — code: NOT NULL, db: nullable
  EXTRA TABLE  legacy_sessions (not declared as a type)

3 issue(s) found, 3 table(s) ok
```

Drift categories:

| Category | Meaning |
|---|---|
| `MISSING TABLE` | A `type` has no corresponding table in the live DB |
| `EXTRA TABLE` | A live table has no `type` declaration (informational) |
| `MISSING COLUMN` | A field exists in the `type` but not in the live table |
| `EXTRA COLUMN` | A live column is not in the `type` declaration |
| `TYPE MISMATCH` | Field type in code differs from the column type in the DB |
| `NULLABLE DRIFT` | Nullability differs between code and DB |

Requires `psql` on `PATH`.

---

## 14. certo migrate — migrations

`certo migrate` is an alias for `certo db` with slightly different subcommand
names. Both accept `--dry-run`.

```sh
certo migrate up              # same as: certo db migrate
certo migrate down [N]        # same as: certo db rollback [N]
certo migrate status          # same as: certo db status
certo migrate create <name>   # same as: certo db create <name>
```

Migration state is tracked in a `.certo_migrations` manifest file in the
project root. Each applied migration is recorded with its name and timestamp.

Migration files are loaded from `migrations/*.cto` in lexicographic order.
Name your files with a numeric prefix to control order:

```
migrations/
    001-create-users.cto
    002-create-orders.cto
    003-add-users-role.cto
```

---

## 15. certo.toml — project manifest

When a `certo.toml` exists in the current directory, `certo build` (and bare
`certo`) reads it for defaults.

### Minimal example

```toml
[project]
name    = "my-app"
version = "0.1.0"
edition = "2026"

[build]
type   = "app"          # "app" or "lib"
target = "native"
output = "dist/"        # output directory (created if absent)
entry  = "src/main.cto" # entry point (required for type = "app")
```

### Library project

```toml
[project]
name    = "my-lib"
version = "0.1.0"
edition = "2026"

[build]
type   = "lib"
target = "native"
output = "dist/"
# no entry — lib has no main
```

When `type = "lib"`, `certo build` automatically passes `--emit-dll`.

### Fields

| Field | Section | Description |
|---|---|---|
| `name` | `[project]` | Project name |
| `version` | `[project]` | SemVer string |
| `edition` | `[project]` | Language edition (e.g. `"2026"`) |
| `type` | `[build]` | `"app"` or `"lib"` |
| `target` | `[build]` | Always `"native"` for now |
| `output` | `[build]` | Output directory; created automatically |
| `entry` | `[build]` | Entry file (required for apps) |

---

## 16. C compiler discovery and PostgreSQL linking

### C compiler

Certo compiles your program by emitting C and invoking a C compiler.
The compiler is found by probing `PATH` in this order:

1. `clang`
2. `gcc`
3. `cc`
4. `cl` (MSVC)
5. `C:\Program Files\LLVM\bin\clang.exe` (Windows fallback)
6. `C:\Program Files (x86)\LLVM\bin\clang.exe` (Windows fallback)

Use `certo build -v` to see the exact command invoked:

```sh
certo build src/main.cto -v
# clang /tmp/certo_abc123.c -o dist/main -O2 -Wno-int-to-pointer-cast ...
```

If no compiler is found:

```
error: no C compiler found (tried cc, gcc, clang)
       install gcc or clang, or use --emit-c to get the C source
```

### PostgreSQL linking

When a program imports `Stdlib.Db`, the compiler automatically links `libpq`.
PostgreSQL include and lib paths are resolved in this order:

1. `PG_INCLUDE` / `PG_LIB` environment variables (always win if set)
2. Windows: probe `C:\Program Files\PostgreSQL\<version>\` (newest first)
3. Unix: run `pg_config --includedir --libdir`
4. Fall back to system search path (works if `libpq-dev` is installed)

**Windows example** — override paths explicitly:

```sh
set PG_INCLUDE=C:\Program Files\PostgreSQL\16\include
set PG_LIB=C:\Program Files\PostgreSQL\16\lib
certo build src/main.cto -o dist/main.exe
```

**Linux** — install the development headers:

```sh
# Debian/Ubuntu
apt-get install libpq-dev

# Fedora/RHEL
dnf install libpq-devel
```

---

## 17. Environment variables

| Variable | Used by | Description |
|---|---|---|
| `DATABASE_URL` | `certo db`, migrations | PostgreSQL connection string |
| `PG_INCLUDE` | `certo build` | Override PostgreSQL include path |
| `PG_LIB` | `certo build` | Override PostgreSQL lib path |
| `NO_COLOR` | All commands | Disable ANSI colour output |
| `TERM` | All commands | `dumb` disables colour |

### DATABASE_URL formats

Both connection string formats are accepted:

```sh
# URI format
DATABASE_URL=postgresql://myuser:secret@localhost:5432/mydb

# Key-value format (libpq connection string)
DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret
```

`DATABASE_URL` is read from the environment first, then from a `.env` file in
the current directory.

### .env file

Place a `.env` file in your project root:

```sh
DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret
PORT=8080
APP_ENV=development
```

The `.env` file is **not** loaded into the process environment automatically
(unlike some frameworks). Only `certo db` and migration commands read it.
Use a shell helper like `dotenv` if you need `.env` loaded for `certo run`.

---

## 18. Exit codes

| Code | Meaning |
|---|---|
| `0` | Success |
| `1` | Compile error, test failure, lint warning, or schema drift |
| `2` | Bad arguments or usage error |
| `N` | Program exit code (from `certo run`) |

---

## 19. CI recipe

A complete GitHub Actions workflow for a Certo project:

```yaml
name: CI

on: [push, pull_request]

jobs:
  build:
    runs-on: ubuntu-latest

    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_USER: ci
          POSTGRES_PASSWORD: ci
          POSTGRES_DB: cidb
        ports: ["5432:5432"]

    steps:
      - uses: actions/checkout@v4

      - name: Install Certo
        run: |
          # Download the latest release binary
          curl -sSL https://certo.dev/install.sh | sh
          echo "$HOME/.certo/bin" >> $GITHUB_PATH

      - name: Install C compiler
        run: sudo apt-get install -y gcc libpq-dev

      - name: Check formatting
        run: certo fmt --check src/*.cto

      - name: Type-check
        run: certo check src/main.cto

      - name: Lint
        run: certo lint src/*.cto

      - name: Build
        run: certo build src/main.cto -o dist/myapp

      - name: Run tests
        run: certo test src/tests.cto
        env:
          DATABASE_URL: postgresql://ci:ci@localhost:5432/cidb

      - name: Migrate database
        run: certo db migrate
        env:
          DATABASE_URL: postgresql://ci:ci@localhost:5432/cidb

      - name: Schema drift check
        run: certo db diff src/domain.cto
        env:
          DATABASE_URL: postgresql://ci:ci@localhost:5432/cidb
```

### Minimal CI (no database)

```yaml
- run: certo fmt --check src/*.cto
- run: certo check src/main.cto
- run: certo lint src/*.cto
- run: certo build src/main.cto -o dist/myapp
- run: certo test src/tests.cto
```

### Typical local workflow

```sh
# Start a new feature
certo new my-feature --template api
cd my-feature

# Develop with live reload
certo run src/main.cto --watch &

# Before committing
certo fmt src/*.cto
certo lint src/*.cto
certo test src/tests.cto

# Apply database changes
certo db create add_sessions_table
# ... edit migrations/add_sessions_table.cto ...
certo db migrate

# Check for drift
certo db diff src/domain.cto

# Build a release binary
certo build src/main.cto -o dist/my-feature
```
