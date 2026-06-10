# Certo Compiler — Backlog

Tasks are listed in implementation order. Completed tasks have the crate they live in noted.

## Completed

| # | Task | Crate |
|---|------|-------|
| 1 | AST data structures | `crates/ast` |
| 2 | Parser | `crates/parser` |
| 3 | Name resolution | `crates/resolve` |
| 4 | Type checker (Algorithm W / HM inference) | `crates/typeck` |
| 5 | Trait system (impl conformance + bound checking) | `crates/traits` |
| 6 | Effect type checker | `crates/effects` |
| 7 | DB schema connection and validation | `crates/dbschema` |
| 8 | HIR and MIR lowering | `crates/hir`, `crates/mir` |
| 9 | C transpiler backend | `crates/codegen` |
| 10 | Stdlib — Core, Collections, Text, DateTime, Money | `crates/stdlib` |
| 11 | Test runner (`certo-test`) | `crates/testrunner` |
| 12 | Migration tool | `crates/migrate`, `crates/cli` |
| 13 | LLVM backend (`certo-llvm`) | `crates/llvm` |
| 14 | LSP server | `crates/lsp` |
| 15 | Formatter (`certo-fmt`) | `crates/fmt` |
| 16 | UI compiler (`certo-ui`) | `crates/ui` |
| 17 | C FFI and REST client generation | `crates/ffi` |
| 18 | WASM target (`certo-wasm`) | `crates/wasm` |
| 19 | C backend end-to-end on Windows (clang, DLL output, `--emit-dll`) | `crates/cli`, `crates/codegen` |
| 20 | `!` boolean-not operator | `crates/parser` |
| 21 | `++` string concat operator | `crates/lexer`, `crates/ast`, `crates/parser`, `crates/codegen`, `crates/hir`, `crates/typeck`, `crates/llvm`, `crates/fmt` |
| 22 | `??` null-coalesce operator wired into parser | `crates/parser` |
| 23 | `form` keyword and parser | `crates/lexer`, `crates/parser` |
| 24 | Pretty error messages with source context | `crates/diagnostics`, `crates/cli`, `crates/testrunner` |
| 25 | Developer guide and release / dist-package scripts | `docs/GUIDE.md`, `scripts/` |
| 26 | `for` loops full-stack | `crates/ast`, `crates/parser`, `crates/hir`, `crates/mir`, `crates/typeck`, `crates/resolve`, `crates/effects`, `crates/fmt` |
| 27 | `match` expressions end-to-end | HIR → MIR → codegen path verified |
| 28 | String interpolation `f"Hello, {name}!"` | `crates/parser`, `crates/hir` |
| 29 | `let` destructuring (tuple + record) | `crates/hir` |
| 30 | Stdlib-aware name resolver | `certo_stdlib::seed_stdlib` wired into CLI via `TypeEnv` |
| 31 | Typeck errors in CLI (`certo build` + `certo check`) | `crates/cli`, `crates/typeck` |
| 32 | README at repo root | `README.md` |
| 33 | GitHub Actions CI | `.github/workflows/ci.yml` |
| 34 | `import` resolver (multi-file) | `crates/cli` |
| 35 | `certo check` — type-check without compiling | `crates/cli` |
| 36 | `certo run` — compile and execute in one command | `crates/cli` |
| 37 | Richer type error messages with labels and notes | `crates/cli`, `crates/diagnostics` |
| 38 | Stdlib.Db — parameterized queries, multi-row results, transactions | `crates/stdlib` |
| 39 | Stdlib.Http — server (Windows/WinSock2, POSIX stub) | `crates/stdlib` |
| 40 | Match guards (`x if x > 0 => ...`) full-stack | `crates/hir`, `crates/mir` |
| 41 | Pipe operator partial application (`a \|> f(b)` → `f(a, b)`) | `crates/hir`, `crates/typeck` |
| 42 | `certo new` — project scaffolding with folder structure and starter files | `crates/cli` |
| 43 | `certo repl` — interactive REPL with banner, two-pass type inference, meta-commands | `crates/cli` |
| 44 | `certo bench` — timing harness for `bench_` functions, reports ns/iter | `crates/cli` |
| 45 | `certo lint` — HIR dataflow pass (unused params/vars, dead writes, unreachable) | `crates/cli`, `crates/hir` |
| 46 | `certo bench` — build number tracking (`CERTO_BUILD_DATE`, `CERTO_BUILD_NUM`) | `crates/cli` |
| 47 | Unit return type fix — `HirFn.ret_ty` inferred from MIR result operand | `crates/mir`, `crates/codegen` |
| 48 | LSP stdlib seeding — `check_module_seeded` so builtins don't appear unbound | `crates/lsp` |
| 49 | LSP position-aware local variable completions — `val`/`var` bindings in function bodies | `crates/lsp` |
| 50 | `.cto` file extension standardised across all tooling and docs (was `.certo`) | all crates |
| 51 | LSP keyword completions — added `guard`, `require`, `ensure`, `defer`, `statemachine`, `spawn`, etc. | `crates/lsp` |
| 52 | `?` error propagation — MIR inline branch, `certo_result_t` runtime, `Ok`/`Err` builtins | `crates/mir`, `crates/codegen`, `crates/typeck` |
| 53 | Doc gap fixes — duplicate §10 heading, broken cross-refs, Neovim `filetypes`, formatter indentation, `Result<T,E>` in types table | `docs/GUIDE.md`, `editors/neovim/certo.lua` |
| 54 | Doc additions — §9 project layout, `certo.toml` manifest schema, `--template` options | `docs/GUIDE.md` |
| 55 | Doc additions — `certo doc`, `certo db/migrate`, `--watch`, `fmt --check`, `test --timeout`, `bench --iterations` | `docs/GUIDE.md`, `README.md` |
| 56 | Fix import resolver using stale `.certo` extension — multi-file `import` failed to find `.cto` files | `crates/cli` |
| 57 | Fix migration loader using stale `.certo` extension | `crates/cli` |
| 58 | `[build].output` from `certo.toml` — `cmd_build` now writes binaries to the configured output directory | `crates/cli` |
| 59 | `certo build` / bare `certo` reads `entry` from `certo.toml` — no explicit file argument required in a project directory | `crates/cli` |
| 60 | `certo db` subcommands — `migrate`, `rollback`, `status`, `create`; `certo migrate` kept as alias | `crates/cli` |
| 61 | `guard` statement HIR lint pass (L005) — literal bool condition, L004 improvement for terminal guards | `crates/cli` |
| 62 | `?` operator in REPL two-pass probe — wraps body in `Result`-returning helper when `?` detected | `crates/cli` |
| 63 | `certo db pull` — introspect live PostgreSQL via `psql`, emit `db/schema.cto` with PascalCase types and camelCase fields | `crates/cli` |

## Pending

| # | Task | Notes |
|---|------|-------|
| 64 | Multiple integer types (`Int8`, `Int16`, `Int32`, `UInt`) | Spec §4.1 defines these; compiler only has `Int` (Int64) |
| 65 | `statemachine` declaration lowering | Keyword and AST stub exist; HIR/codegen not implemented |
| 66 | Named function arguments (`f(page: 2, size: 50)`) | Spec §3.2; parser does not yet support named args at call sites |
| 67 | `async`/`await` runtime | Keywords and AST nodes exist; coroutine scheduler not implemented |

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → clang → binary
 1       2        3         4       8     8         9
```

The core pipeline (parse → HIR → MIR → C → binary) is fully operational.
Stdlib names are pre-seeded into the type environment; `Ok`/`Err` are built-in.
`certo build` reads `entry` and `output` from `certo.toml`; `certo db` subcommands are wired.
