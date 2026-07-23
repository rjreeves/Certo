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
| 66 | Named function arguments (`f(page: 2, size: 50)`) — parser, HIR reordering, and typeck all wired; stdlib param metadata registered for ~50 functions | `crates/hir`, `crates/stdlib`, `crates/typeck` |
| 65 | `statemachine` declaration lowering — state enum, struct, constructor, typed transitions with guards, state predicates, `on_enter` hooks inlined | `crates/parser`, `crates/hir`, `crates/typeck`, `crates/codegen` |
| 67 | `async`/`await`/`spawn`/`parallel` — `spawn` keyword, HIR `Spawn`/`Await` nodes, MIR single-threaded stub, resolve/typeck/effects/fmt all wired; runtime header has pthread scaffolding for future real threading | `crates/lexer`, `crates/parser`, `crates/ast`, `crates/hir`, `crates/mir`, `crates/typeck`, `crates/resolve`, `crates/effects`, `crates/fmt`, `crates/codegen` |
| 68 | DB query DSL v1 — `Query.from`/`.filter`/`.orderBy`/`.limit`/`.offset`/`.sql`/`.list`/`.first`/`.count`; table/column/operator/direction literals verified against schema at compile time (E0508-E0512) | `crates/dbschema`, `crates/stdlib`, `crates/cli`, `crates/codegen` |
| 69 | Joins — `Query.join`/`.leftJoin("Table", "Base.col", "Table.col")`. ON columns must be qualified (E0518); qualifier must name the base table or an already-joined table (E0517); a bare `.filter`/`.orderBy` column ambiguous across joined tables is rejected (E0515). Self-joins (same table twice with an alias) not supported yet. | `crates/dbschema`, `crates/stdlib` |
| 70 | Aggregations — `Query.groupBy`/`.aggregate("count"\|"sum"\|"avg"\|"min"\|"max", col, alias)`/`.having`, read via `.groupedList` (no `DbRow` bound — result shape is synthetic); plus ungrouped scalar `Query.sum`/`.avg`/`.min`/`.max`. `.list`/`.first`/`.count`/scalar aggregates are rejected at compile time on an already-grouped/aggregated query (E0516), since they assume `SELECT *`/single-scalar shape. | `crates/dbschema`, `crates/stdlib` |
| 71 | Mutation builder — `Mutation.insertInto`/`.updateTable`/`.deleteFrom`/`.insertMany(table, columns)`/`.set`/`.filter`/`.onConflict`/`.addRow`/`.run`. Table/column/operator/column-list literals verified against schema at compile time (E0519, reusing E0509-E0511 from `Query`); each method restricted to the mutation kinds it's valid for — `.set` on insert/update, `.filter` on update/delete, `.onConflict` on insert (upsert), `.addRow` on insertMany — misuse rejected at compile time (E0520) rather than failing at the database; `.addRow`'s arity checked against `.insertMany`'s declared columns when both are literal (E0521). `insertMany` batches all rows into one round trip. | `crates/dbschema`, `crates/stdlib`, `crates/cli`, `crates/codegen` |
| 72 | Live schema-sync — opt-in via `[database] schema-sync = true` in `certo.toml` (reads `DATABASE_URL` like `certo db pull`, requires `psql` on PATH). Cross-checks every `type X = {...}` with `impl DbRow for X {}` against a live `information_schema.columns` snapshot: missing table (E0522), missing column (E0523), type mismatch (E0524), nullability mismatch (E0525). Table/column names bridged via the same `snake_case` ↔ `PascalCase`/`camelCase` convention `certo db pull` already uses. Off by default — a normal `certo build`/`check` never touches the network; only `DbRow`-annotated types are checked, not every declared record. | `crates/dbschema`, `crates/cli` |
| 69b | Self-joins — `Query.fromAs`/`.joinAs`/`.leftJoinAs(table, alias, ...)` give a table an explicit alias, so the same table can appear twice (e.g. employees joined to their own managers). Alias must be a valid identifier (E0514) and not already in use in this query (E0526) — which is also what makes a plain, non-aliased `.join` of the same table twice a clear compile error instead of silently broken SQL. `.from`/`.join`/`.leftJoin` are unchanged for the common case (alias defaults to the table name). | `crates/dbschema`, `crates/stdlib`, `crates/cli`, `crates/codegen` |
| 97 | Parser bug fixed — parenthesized arrow-lambda expressions (`(x) => expr`, `(a, b) => expr`, `(a: Int) => expr`, `() => expr`) didn't parse **at all**, not just the multi-param case as first logged — the spec's own single-param example (`List.map(xs, (x) => x * 2)`) was broken too. Root cause: `parse_atom` had zero lookahead for this form; only the trailing-block `{ x => }` and explicit `fn(x: T): R = ...` lambda syntaxes existed. Fixed with a balanced-paren scan (`peek_is_arrow_lambda`) that detects `=>` immediately after a matching `)` — unambiguous, since a parenthesized value/tuple is never legally followed by `=>` anywhere else in the grammar. 7 new parser tests (single/multi/typed/zero-param/curried lambdas, plus a regression guard that ordinary `(expr)`/tuples still parse). | `crates/parser` |
| 98 | Typeck + codegen bug fixed — `List.empty()`/`Map.empty()` failed to type-check (misreported as an unapplied `() => U` function). Root cause: both were registered in `seed.rs` as bare polymorphic *values* (`Forall<T>. List<T>`) instead of zero-arg *functions* (`Forall<T>. () => List<T>`) — every other zero-arg stdlib function was correctly wrapped, these two were the only exceptions. Fixing the registration then exposed a second, latent codegen bug: `certo_list_empty` was a `#define certo_list_empty certo_list_new_empty()` macro assuming bare-reference use, which double-expanded once codegen correctly started emitting a real call `certo_list_empty()`. Replaced the macro with real `certo_list_empty`/`certo_map_empty` C functions. 2 new stdlib regression tests asserting the registered type is actually `Fn{params: [], ..}`, not a bare value. | `crates/stdlib`, `crates/codegen` |
| 99 | Type-level bug found while testing #97/#98 — multi-param function *type* annotations `(Int, Int) => Int` parse the parenthesized group as a single tuple-typed parameter (`((Int, Int)) => Int`), not two params, because `parse_type.rs`'s `A => B` sugar always wraps the LHS as one param regardless of whether it's itself a parenthesized list. Not yet fixed — workaround is to omit the annotation and let inference figure out the (single, lambda-expression-level) param types. `crates/parser` |
| 64 | Multiple integer types (`Int8`, `Int16`, `Int32`, `UInt`) — added to `Ty` enum, unification, display, typeck name lookup, C codegen (`int8_t`/`int16_t`/`int32_t`/`uint64_t`), LLVM backend, and REPL auto-print | `crates/typeck`, `crates/codegen`, `crates/llvm`, `crates/cli` |
| 100 | Row polymorphism — `fn getName<R: { name: Text }>(record: R): Text = record.name`. AST gained a `Bound` enum (`Trait`/`Row`) so a type param can carry a record-shape bound alongside/instead of a trait bound; parser accepts `{ field: Ty, ... }` in the bound position. Bound satisfaction is checked as a post-hoc pass at call sites (direct calls and pipe calls both go through `resolve_callee`/`apply_row_check` in `infer_expr.rs`), mirroring the existing trait-bound-checking architecture rather than baking row constraints into core unification — the row-bounded param is instantiated and unified as an ordinary fresh type variable, then afterward its resolved type is checked against the required fields (present + unifiable) with a new E0212 diagnostic for a missing field. Works for both exact-shape and superset-shape record arguments. **Known v1 limitation:** only bare-name calls (`Expr::Path`, e.g. `getName(x)`) and pipes are covered — `Type.method(...)` calls parse as `Expr::Field` and are not yet routed through the row-bound check. | `crates/ast`, `crates/parser`, `crates/fmt`, `crates/traits`, `crates/typeck`, `crates/cli` |

## Pending

_Added 2026-07-16 after a full audit of `docs/Certo_Language_Specification.md` v0.1 against the actual codebase.
The spec is a vision/pitch document — the items below are real, verified gaps between it and what compiles
today (see audit method note at the bottom of this section). Ordered by priority._

### DB query DSL (highest priority — the language's core differentiator)

Items 68-72 and 69b (builder v1, joins, aggregation, mutations, live schema-sync,
self-joins) are done — see the Completed table above. What's left:

| # | Task | Crate |
|---|------|-------|
| 72b | Full live schema connection (compiler reads `information_schema` unconditionally, replacing declared `type`s as the source of truth, not just a `DbRow`-gated opt-in drift check) | Not started — this is the more invasive version the spec describes; #72 deliberately scoped to an opt-in cross-check instead, to avoid making every build require a database. Revisit only if there's a concrete need. |

### Type system

| # | Task | Notes |
|---|------|-------|
| 73 | Match exhaustiveness checking | Spec claims match arms are checked for exhaustiveness; not enforced anywhere in `typeck`. |
| 74 | `priv` constructors (smart-constructor pattern) | `Priv` token is lexed but never consumed by the parser — no effect today. |
| 75 | Parameterized `Decimal(p,s)`, `Float32`, `Char` | `Decimal` has no precision/scale; no `Float32`/`Char` type exists. |
| 76 | Higher-kinded types (`Functor<F<_>>`) | Not started. Row polymorphism (`<R: { name: Text }>`) is done — see item 100 in the Completed table above, plus item 101 below for the `Type.method(...)` call-site gap it left open. HKT is a materially larger change than row polymorphism was: `Ty` today has no notion of a *type constructor* (`F` in `Functor<F<_>>`) as a value distinct from a fully-applied type, so `F<Int>`/`F<Text>` can't even be represented, let alone unified against each other. A real implementation needs: (1) a kind system — at minimum `Type` vs `Type -> Type`, tracked per type-param so `F` can be checked as a 1-arg constructor rather than a concrete type; (2) a `Ty::App(Box<Ty>, Box<Ty>)` (or similar) variant so `F<Int>` is `App(Var(F), Named("Int"))` instead of requiring `F` to already be a concrete `Named`; (3) restricted higher-order unification — full higher-order unification is undecidable in general, but it's tractable here because the only thing ever unified against a kind-`Type -> Type` variable is a *known* constructor (a user's `type Box<T> = ...` or a builtin like `List`), never an arbitrary lambda, so unification can special-case "variable applied to arg" against "known constructor applied to arg" and decompose structurally. None of this can be bolted onto the row-polymorphism post-hoc-check pattern — it has to live in `crates/typeck/src/ty.rs`/`unify.rs` itself, which is why it was scoped out of this session rather than attempted as a partial slice. |
| 101 | Row bounds don't check `Type.method(...)` call sites | Row polymorphism (item 100) only checks bare-name calls (`getName(x)`) and pipes (`x \|> getName`) — `TypeName.method(...)` parses as `Expr::Field { expr: Path("TypeName"), field: "method" }`, not a 2-segment `Expr::Path`, so `resolve_callee` in `crates/typeck/src/infer_expr.rs` never looks it up in `row_bounds` for that call shape. A row-bounded impl method called via dot syntax on its type name currently type-checks without the bound being enforced. |
| 77 | Result combinators — `flatMap`/`mapErr`/`getOrElse`/`recover`/`Result.all`/`Result.allSettled` | Only `?` propagation and `match` exist today. |
| 99 | Multi-param function *type* annotations `(Int, Int) => Int` misparse as a single tuple-typed param | See item 99 in the Completed table above for the full root cause — found as a byproduct of fixing #97/#98, not yet fixed itself. |

### Security

| # | Task | Notes |
|---|------|-------|
| 78 | `Secret<T>` type (uncloggable/unserializable) | Not started. |
| 79 | Phantom-type authorization (`Permission<T>`) | Not started. |

### Concurrency

| # | Task | Notes |
|---|------|-------|
| 80 | `Channel<T>`, `withTimeout`, supervised `spawn`/`every()` | `spawn`/`await`/`defer` are real (OS threads); these extras are not started. |
| 81 | Enforce `parallel(timeout:)` | Currently parsed and silently dropped — not enforced in codegen. |
| 82 | `statemachine` `on_enter`/`invariant` codegen | Parsed but explicitly deferred per `docs/LIMITATIONS.md`. |

### Stdlib

| # | Task | Notes |
|---|------|-------|
| 83 | Collections: `groupBy`/`chunked`/`sumBy`/`minBy`/`maxBy`/`partition`/`distinct` | Missing from `crates/stdlib/src/collections.rs`. |
| 84 | Text: `byteLength`/`toDecimal`, locale-aware `toUppercase` | Missing/partial. |
| 85 | `Duration` type, `Timezone`, timezone-aware `DateTime` | DateTime is UTC-only today, no `Duration`/`Timezone`. |
| 86 | True property-based testing (`forAll` with generation/shrinking) | `property` blocks run once today, no generator. |

### UI compiler

| # | Task | Notes |
|---|------|-------|
| 87 | `@ui.generate` schema-driven CRUD scaffolding | Not started. |
| 88 | `live val` reactive queries | Parser stub only (`view.rs` emits a placeholder comment). |
| 89 | React/PWA/Flutter UI targets | Htmx is the only target; no plans to add the others without a specific customer need. |

### Toolchain

| # | Task | Notes |
|---|------|-------|
| 90 | `certo.toml` `[features]`/`[targets.production]` schema parsing | No toml schema parser exists at all yet — `new` only writes a template string. |
| 91 | `certo add`/`certo audit` | Not started. |
| 92 | `run --watch --port`, `check --strict --explain`, `test --filter --coverage` | Flags not parsed. |

### Interop

| # | Task | Notes |
|---|------|-------|
| 93 | `@apiClient` OpenAPI-spec client generation | `ffi::rest` generates from a custom JSON schema, not OpenAPI, and there's no `@apiClient` annotation. |
| 94 | `@export("name")` decorator | Not started — `pub fn` already auto-generates a C header, just not via this annotation. |

### Housekeeping

| # | Task | Notes |
|---|------|-------|
| 95 | Wire `certo-effects` into the CLI pipeline | Effect checking exists as a real, tested crate but is **not invoked anywhere in `certo build`/`certo check`** today — `[pure]`/`[db.write]`/etc. annotations are currently unenforced at compile time. `docs/CONTRIBUTING.md` describes it as wired in; that's stale. |
| 96 | `crates/xeq` — unrelated Windows launcher utility sitting in the workspace | Not mentioned anywhere in this backlog; confirm whether it belongs in this repo. |
| 102 | Wire `check_call_bounds` into the CLI pipeline | `crates/traits/src/bounds.rs` defines and exports `check_call_bounds` (trait-bound satisfaction checking at call sites) but it is **never actually invoked anywhere in the compiler pipeline** — found while wiring row-polymorphism bound checking (item 100) alongside it. Trait bounds on generic functions (e.g. `fn f<T: Comparable>(x: T)`) are effectively unenforced at call sites today, the same class of gap as item 95 (`certo-effects` not wired in). |

_Audit method: three parallel code-vs-spec sweeps across all crates, cross-checked with `docs/LIMITATIONS.md` and direct grep/read of the relevant parser/typeck/codegen/stdlib source. Items above are things confirmed absent or stubbed in code, not just absent from this file._

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → clang → binary
 1       2        3         4       8     8         9
```

The core pipeline (parse → HIR → MIR → C → binary) is fully operational.
Stdlib names are pre-seeded into the type environment; `Ok`/`Err` are built-in.
`certo build` reads `entry` and `output` from `certo.toml`; `certo db` subcommands are wired.
