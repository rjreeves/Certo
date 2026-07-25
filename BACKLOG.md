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
| 103 | Bug fixed — `??` (null-coalesce) was left-associative, breaking natural fallback chains | Found while writing a `pg_version.cto` exercise: `getEnv("DATABASE_URL") ?? arg(1) ?? default` failed to typecheck because `a ?? b ?? c` parsed as `(a ?? b) ?? c`, and typeck requires `??`'s right operand to be non-optional — but `b` (`arg(1)`) is itself `Text?`. Independently confirmed as a real, previously-hit trap: `examples/runsql.cto` already had a manual-parens workaround for the identical pattern (`arg(2) ?? (getEnv("PG_CONNSTR") ?? "")`). Every other language with a null-coalescing operator (C# `??`, Kotlin `?:`, Swift `??`) makes it right-associative for exactly this reason. Fixed `parse_coalesce` in `crates/parser/src/parse_expr.rs` to recurse on the right instead of looping, mirroring the existing right-associative `**` (`parse_power`) pattern — `a ?? b ?? c` now parses as `a ?? (b ?? c)`. 2 new parser tests (chain is right-associative; single `??` still parses) and 1 new typeck test (a 3-level chain with an optional middle value type-checks). | `crates/parser` |
| 95 | Wired `certo-effects` into `certo build`/`certo check` | `run_typeck` in `crates/cli/src/main.rs` now calls `certo_effects::check_module` right after the trait-bound check, converting `EffectError`s into E0400-E0404 diagnostics. Verified against every example `.cto` (no false positives) and against hand-written violations of each rule (undeclared effect, `await` outside `[async]`, `unsafe {}` outside `[unsafe]`) — all now correctly rejected. **Real limitation surfaced by turning this on:** `certo_effects::build_env` only scans the *current module's own* `fn`/`impl`/`trait` declarations (`crates/effects/src/effect_env.rs`) — it has no equivalent of `certo_stdlib::seed_stdlib`, so calls to stdlib functions (`println`, `dbConnect`, `dbExec`, etc.) contribute **no inferred effect at all**. A `[pure]` function that calls `println`/`dbExec` directly is silently accepted — the checker only catches effect violations that flow through calls to other functions declared in the same module, or through `await`/`unsafe {}`/`db.transaction {}` syntax. This is the same class of gap as row polymorphism's `Type.method(...)` limitation: real, but scoped out of "wire it in." See item 105. | `crates/cli`, `crates/effects` |
| 102 | Wired trait-bound checking at generic call sites (`check_call_bounds`) into `certo build`/`certo check` | `check_call_bounds` (item was previously dead code) is now driven by a new best-effort static walker, `check_generic_call_bounds` in `crates/traits/src/bounds.rs`, wired into `certo_traits::check_module` (already invoked by the CLI, so no CLI changes were needed for this half). For every call `f(args...)` to a module-local function/impl-method with a `Trait`-bounded type param `T`, where the argument at `T`'s position is a record literal `TypeName { .. }`, verifies `TypeName` satisfies the bound — same "best-effort, syntactic, no inference" philosophy as the pre-existing `check_dbquery_typed_bounds`, whose recursive AST walker was refactored into a shared, reusable `for_each_call`/`for_each_body` pair (both checks now use it, replacing two near-duplicate ~130-line walkers with one). 4 new tests: satisfying/unsatisfying record-literal arguments, an impl method (also collected, though only reachable once item 101 fixes dot-call routing), and confirmation that a non-literal (variable) argument is correctly *not* flagged (no false positives). **Same v1 limitation as row polymorphism (item 101):** only bare-name calls are checked, not `Type.method(...)` dot syntax. | `crates/traits` |
| 106 | Bug fixed — two `certo-effects` diagnostic-quality bugs, invisible until item 95 wired the checker into the CLI | (1) `EffectErrorKind::ImpureCallInPure`'s `callee` field was always constructed as `String::new()`, so the diagnostic read `pure function `f` calls ``, which requires ...` with empty backticks. (2) The same violation fired **two** diagnostics for one root cause — a generic per-effect branch and a separate "pure function calling any effectful callee" sweep both triggered, emitting E0400 and E0401 for the same span. Root cause of both: `InferredEffects::origins` (`crates/effects/src/infer_effects.rs`) never tracked *which named function* introduced an inherited effect, and `check_fn_body` (`crates/effects/src/check_effects.rs`) ran two independent, overlapping passes over the same origins. Fixed by adding `callee: Option<String>` to each origin (`Some(name)` for a call whose declared effects were inherited, `None` for a direct syntactic form like `await`/`unsafe {}`/`db.transaction {}`), and consolidating to a single pass: a pure function's call to a named effectful callee now gets exactly one `ImpureCallInPure` with the real callee name; a direct syntactic effect gets exactly one of the existing specific messages (`UnsafeOutsideUnsafe`/`TransactionOutsideDbWrite`/etc.), never both. Also fixed a latent third bug found in the same pass: the old generic branch had no dedicated `Fallible` arm, so `?` inside a `[pure]` function was incorrectly flagged even though the crate's own design intent (stated in a comment) was to allow it — now explicitly skipped alongside `Pure`. 1 new test asserting the real callee name and exactly one error; 1 existing test's assertion narrowed from "either generic kind" to the specific, now-deduplicated `UnsafeOutsideUnsafe`. | `crates/effects` |
| 105 | Wired stdlib effect-seeding into `certo_effects` — `[pure]` functions calling stdlib I/O are now checked | New `certo_stdlib::seed_stdlib_effects` (`crates/stdlib/src/effects_seed.rs`) hand-registers `Effect::Io`/`Effect::Fallible` for every genuinely effectful stdlib function (`println`, `dbConnect`, `dbExec`, `readFile`, `Http.get`, etc.), and `certo_effects` gained `build_env_seeded`/`check_module_seeded` (mirroring `certo_typeck::check_module_seeded`) so a pre-seeded environment can be merged with the module's own declarations. Wired into `run_typeck` in `crates/cli/src/main.rs`. `fn f(): Int [pure] = { println("x"); 1 }` now correctly produces E0401 naming `println` as the offending callee — the exact motivating case for item 95. **Hand-maintained, not derived from source:** originally attempted to parse `certo_stdlib::certo_sources()` (the `*_CERTO` doc-text constants) directly instead of hand-listing function names, since those already carry `[io]` annotations — but most of those constants aren't valid, parseable Certo (see item 107), so the names/effects here were extracted from that text by hand instead. Dot-qualified names (`Query.list`, `Http.get`, ...) are registered too, even though they're not reachable yet at `Type.method(...)` call sites — same limitation as items 101/102. 2 new tests (known I/O functions get an entry; known-pure functions don't). | `crates/effects`, `crates/stdlib`, `crates/cli` |

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
| 96 | `crates/xeq` — unrelated Windows launcher utility sitting in the workspace | Not mentioned anywhere in this backlog; confirm whether it belongs in this repo. |
| 107 | `certo_stdlib::certo_sources()` returns unparseable pseudo-Certo for most modules, and is otherwise dead code | Found while implementing item 105 — the original plan was to derive stdlib effect annotations by parsing these `*_CERTO` constants (they already carry accurate `[io]`/`[fallible]` annotations as text) instead of hand-listing function names. Ran `certo_parser::parse()` against every entry from `certo_sources()`: only `Stdlib.Core` and `Stdlib.File` actually parse. Every other module fails, mostly because the source text declares functions with a `fn TypeName.method(...)` header (e.g. `fn List.len<T>(list: List<T>): Int` in `COLLECTIONS_CERTO`) — valid as a qualified *string key* in `seed.rs`'s hand-written `TypeEnv` registration, but not a declaration form the real parser grammar supports (`fn` only accepts a bare identifier). `Stdlib.Db` additionally fails because its `trait DbRow {}` marker is written before the `module` line. `Stdlib.Math`/`Crypto`/`Regex`/`Csv` fail on `extern` blocks. Nothing in the codebase actually consumes `certo_sources()` for a real purpose today (`grep` shows only its own tests, which just check non-emptiness) — it appears to be intended as either human-readable API documentation text or scaffolding for a future `certo doc`/parser feature that was never finished, not genuine parseable source. Worth either fixing (support `fn Type.method(...)` declaration syntax, if that's the intended long-term surface) or removing/relabeling as documentation-only so it doesn't mislead a future reader into assuming it's real source, as it did here. |
| 104 | MSVC `cl.exe` is listed as a supported C-compiler candidate but is never actually usable | Found while compiling a real program on a machine with only Visual Studio installed (no clang/gcc). Two separate bugs, both in `crates/cli/src/main.rs`: (1) `probe_cc("cl")` runs `cl --version`, but `cl.exe` doesn't support `--version` — it prints a warning, then errors with D8003 ("missing source filename") and exits 2, so the probe reports `cl` absent even when it's on `PATH`/found via vcvars. (2) Even if detection were fixed, `cmd_build`'s compiler invocation (`main.rs:489-564`, and the near-identical one in `cmd_bench` around `main.rs:1913-1922`) is 100% GCC/Clang-flag syntax (`-o`, `-I`, `-Xlinker`, `-luser32`) — none of which `cl.exe` understands (it wants `/Fe:`, `/I`, `/link`, `user32.lib`). Net effect: `certo build`/`run`/`bench` cannot produce an executable at all on a Windows machine that has only MSVC and no LLVM/MinGW. Worked around this session by installing LLVM via `winget install LLVM.LLVM` instead of fixing MSVC support; a real fix would need a parallel `cl.exe` argument-translation path in both call sites (or removing `"cl"` from the `find_cc` candidate list entirely, if MSVC support isn't intended to work). |

_Audit method: three parallel code-vs-spec sweeps across all crates, cross-checked with `docs/LIMITATIONS.md` and direct grep/read of the relevant parser/typeck/codegen/stdlib source. Items above are things confirmed absent or stubbed in code, not just absent from this file._

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → clang → binary
 1       2        3         4       8     8         9
```

The core pipeline (parse → HIR → MIR → C → binary) is fully operational.
Stdlib names are pre-seeded into the type environment; `Ok`/`Err` are built-in.
`certo build` reads `entry` and `output` from `certo.toml`; `certo db` subcommands are wired.
