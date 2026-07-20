# Contributing to Certo — Compiler Internals Guide

Everything a new contributor needs to understand the codebase, make a change,
and get it reviewed.

---

## Table of Contents

1. [Repository layout](#1-repository-layout)
2. [Building and testing](#2-building-and-testing)
3. [Compiler pipeline overview](#3-compiler-pipeline-overview)
4. [Crate-by-crate reference](#4-crate-by-crate-reference)
   - [certo-lexer](#certo-lexer)
   - [certo-ast](#certo-ast)
   - [certo-parser](#certo-parser)
   - [certo-resolve](#certo-resolve)
   - [certo-typeck](#certo-typeck)
   - [certo-traits](#certo-traits)
   - [certo-effects](#certo-effects)
   - [certo-dbschema](#certo-dbschema)
   - [certo-hir](#certo-hir)
   - [certo-mir](#certo-mir)
   - [certo-codegen](#certo-codegen)
   - [certo-stdlib](#certo-stdlib)
   - [certo-migrate](#certo-migrate)
   - [certo-fmt](#certo-fmt)
   - [certo-diagnostics](#certo-diagnostics)
   - [certo-testrunner](#certo-testrunner)
   - [certo-llvm](#certo-llvm)
   - [certo-lsp](#certo-lsp)
   - [certo-cli](#certo-cli)
   - [Other crates](#other-crates)
5. [Key data structures](#5-key-data-structures)
6. [How to add a built-in function](#6-how-to-add-a-built-in-function)
7. [How to add a stdlib module](#7-how-to-add-a-stdlib-module)
8. [How to add a new keyword / syntax](#8-how-to-add-a-new-keyword--syntax)
9. [How to add a compiler error](#9-how-to-add-a-compiler-error)
10. [How to add a lint rule](#10-how-to-add-a-lint-rule)
11. [Testing conventions](#11-testing-conventions)
12. [Diagnostics and error formatting](#12-diagnostics-and-error-formatting)
13. [Code style](#13-code-style)

---

## 1. Repository layout

```
Certo/
├── Cargo.toml              workspace root
├── crates/
│   ├── lexer/              Token enum (logos-based)
│   ├── ast/                Untyped syntax tree (spans everywhere)
│   ├── parser/             AST → text parser
│   ├── resolve/            Name resolution pass
│   ├── typeck/             Hindley-Milner type inference
│   ├── traits/             Trait / impl checking
│   ├── effects/            Effect annotation checking
│   ├── dbschema/           Migration ↔ type consistency checks
│   ├── hir/                High-level IR + AST→HIR lowering
│   ├── mir/                Mid-level IR (basic blocks) + HIR→MIR lowering
│   ├── codegen/            MIR → C code emitter
│   ├── stdlib/             Stdlib C implementations + Certo signatures
│   ├── migrate/            Migration runner (apply / rollback / status)
│   ├── fmt/                Source formatter
│   ├── diagnostics/        Diagnostic struct + ANSI renderer
│   ├── testrunner/         test block runner
│   ├── llvm/               Experimental LLVM IR emitter
│   ├── lsp/                Language Server Protocol server
│   ├── ffi/                FFI bridge helpers
│   ├── ui/                 View / form compiler
│   ├── wasm/               WebAssembly backend
│   ├── xeq/                Bytecode interpreter (experimental)
│   └── cli/                `certo` binary — wires everything together
├── docs/                   Documentation
└── dist/                   Compiled binaries (gitignored)
```

The crates form a DAG — the dependency order is shown in §3.

---

## 2. Building and testing

### Prerequisites

- Rust stable (1.75+) — install via [rustup](https://rustup.rs)
- A C compiler (`clang`, `gcc`, or MSVC `cl`) — needed by integration tests
  that compile the generated C
- PostgreSQL client tools (`psql`, `libpq-dev`) — needed only if working on
  `certo-stdlib/db.rs` or `certo-cli/cmd_db`

### Build everything

```sh
cargo build                    # debug
cargo build --release          # release (slow compile, fast binary)
```

### Run all tests

```sh
cargo test                     # all crates
cargo test -p certo-codegen    # single crate
cargo test -p certo-typeck -- --nocapture   # with stdout
```

### Run the CLI from source

```sh
cargo run -p certo-cli -- build src/main.cto -o out
cargo run -p certo-cli -- run src/main.cto
```

### Check without compiling

```sh
cargo check                    # type-check all crates, no codegen
```

---

## 3. Compiler pipeline overview

```
Source text (.cto)
       │
       ▼
┌─────────────┐
│  certo-lexer │  Token stream (logos, zero-copy borrows from source)
└─────────────┘
       │
       ▼
┌──────────────┐
│ certo-parser │  → Module (AST, spans attached to every node)
└──────────────┘
       │
       ├──► certo-resolve     Name resolution  (E0100–E0102, E0700–E0702)
       │
       ├──► certo-typeck      Type inference   (E0200–E0206, E0708–E0709)
       │          │
       │          └──► certo-traits   Trait checking   (E0300–E0306)
       │
       ├──► certo-effects     Effect checking  (E0400–E0404)
       │
       ├──► certo-dbschema    Migration checks (E0500–E0507)
       │
       ▼
┌───────────┐
│ certo-hir │  AST → HIR (desugar, type-annotate, name-resolve locals)
└───────────┘
       │
       ▼
┌───────────┐
│ certo-mir │  HIR → MIR (basic blocks, explicit control flow)
└───────────┘
       │
       ├──► certo-codegen     MIR → C  (primary backend)
       │
       └──► certo-llvm        MIR → LLVM IR  (experimental)

certo-stdlib  — C implementations + Certo signatures (injected at build time)
certo-cli     — orchestrates every step; calls the C compiler on the output
```

Every pass takes a `Module` or its result type and is independent of the
others at the type level. The CLI in `crates/cli/src/main.rs` calls them in
order and aborts on the first phase that reports errors.

---

## 4. Crate-by-crate reference

### certo-lexer

**Location:** `crates/lexer/src/lib.rs`

Uses the [logos](https://docs.rs/logos) crate. The entire lexer is a single
`Token<'src>` enum with `#[regex]` and `#[token]` attributes. String slices
borrow from the source — no allocation during lexing.

**Key tokens:**

| Variant | Example | Notes |
|---|---|---|
| `Integer` | `42`, `0xFF`, `0b1010` | Decimal, hex, binary, octal |
| `Float` | `3.14`, `1e9` | Must include `.` or exponent |
| `Decimal` | `d"19.99"` | Prefix `d"..."` for exact decimal |
| `FString` | `f"hello {name}"` | Interpolation resolved in parser |
| `StringLit` | `"hello"` | Standard string |
| `MultilineString` | `"""..."""` | Triple-quoted |
| `UuidLit` | `uuid"550e8400-..."` | UUID literal |
| `Ident` | `myVar` | Identifiers |

Keywords (`fn`, `if`, `match`, `type`, `val`, ...) appear before `Ident`
in the enum so logos gives them higher priority.

**To add a token:** add a new variant with `#[token("...")]` or
`#[regex("...")]`. Put it above the `Ident` variant if it starts with letters
so it wins over identifier matching.

---

### certo-ast

**Location:** `crates/ast/src/`

The untyped syntax tree. Every node is wrapped in `S<T>` — a span-carrying
newtype — so source locations are never lost.

```
ast/src/
    span.rs     S<T> newtype; Span = {start, end} byte offsets
    types.rs    TypeExpr, EffectSet, ModulePath, Ident
    expr.rs     Expr, Stmt, Lit, BinOp, UnOp, FStringPart
    pattern.rs  Pattern
    decl.rs     Decl (Fn, Type, Val, Trait, Impl, StateMachine,
                      Validator, Migration, Test, ...)
    module.rs   Module = { name, imports, decls }
```

**`Decl` variants** (complete list as of the current codebase):

`Fn` · `Type` · `Val` · `Var` · `Trait` · `Impl` · `StateMachine` ·
`Validator` · `Constraint` · `Temporal` · `RuleTest` · `ValidatorTest` ·
`Migration` · `View` · `Form` · `Test` · `Property` · `DbTest` · `Import`

Adding a new declaration form means adding a new `Decl` variant here and
matching on it everywhere `Decl` is pattern-matched (the Rust compiler will
tell you every site).

---

### certo-parser

**Location:** `crates/parser/src/`

A hand-written recursive-descent parser. The public entry point is:

```rust
pub fn parse(src: &str) -> Result<Module, Vec<ParseError>>
```

Internal modules:

| File | Responsibility |
|---|---|
| `cursor.rs` | Token cursor wrapping the lexer |
| `parse_module.rs` | Top-level: parse module declaration and declarations |
| `parse_decl.rs` | Parse each `Decl` variant |
| `parse_expr.rs` | Pratt parser for expressions (operator precedence) |
| `parse_type.rs` | Parse `TypeExpr` |
| `parse_pattern.rs` | Parse `Pattern` |
| `error.rs` | `ParseError` and `ParseErrorKind` |

**Adding syntax:** implement a `parse_<thing>` function in the appropriate
module, call it from `parse_decl.rs` or `parse_expr.rs`, and emit an AST
node. Follow the existing `expect(Token::X)?` pattern.

**Operator precedence** is controlled by the Pratt table in `parse_expr.rs`.
Each binary operator has a binding power (left and right). Higher binding
power = tighter grouping.

---

### certo-resolve

**Location:** `crates/resolve/src/`

Name resolution pass. Walks the AST and checks:
- Every name is declared before use (E0100)
- No conflicting imports (E0101)  
- No duplicate definitions in scope (E0102)
- Validator `after` / `overrides` reference existing rules (E0701, E0702)
- No cycle in the validator rule DAG (E0700)

The key function is:

```rust
pub fn resolve_module(module: &Module) -> Result<(), Vec<ResolveError>>
```

Uses a `Scope` struct that is a stack of `HashMap<String, Span>` frames —
one frame per lexical scope.

---

### certo-typeck

**Location:** `crates/typeck/src/`

Hindley-Milner type inference with let-polymorphism.

```
typeck/src/
    ty.rs          Ty enum (Int, Float, Text, Option, Result, List,
                            Map, Tuple, Named, Record, Fn, Var, Forall, Error)
    env.rs         TypeEnv: name → Ty mapping; seed_builtins() populates it
    unify.rs       Robinson unification; produces substitution
    infer_expr.rs  Infer types for expressions
    infer_decl.rs  Infer types for declarations; drives the whole pass
    error.rs       TypeError and TypeErrorKind
```

**Entry point:**

```rust
pub fn check_module_seeded(
    module: &Module,
    env: TypeEnv,      // pre-seeded with builtins + stdlib
    counter: u32,      // fresh type variable counter
) -> Result<(), Vec<TypeError>>
```

**Type variable lifecycle:**
1. A fresh `Ty::Var(n)` is allocated (counter incremented) for each
   expression without a known type.
2. Unification fills in variables via a substitution map.
3. Generalisation wraps unconstrained variables in `Ty::Forall`.
4. Instantiation replaces `Forall` vars with fresh `Var`s at each use site.

**`Ty::Error`** is a sentinel that suppresses cascading errors. When inference
fails for an expression, it returns `Ty::Error` and records the error; later
passes that see `Ty::Error` skip their own checks.

**Adding a new type:** add a variant to `Ty`, handle it in `unify.rs` and
everywhere `Ty` is matched. The Rust compiler will enumerate every unhandled
site.

---

### certo-traits

**Location:** `crates/traits/src/`

Checks that every `impl Trait for Type` block:
- Contains exactly the methods the trait declares (E0301 = missing, E0300 = extra)
- Has matching parameter counts (E0302), return types (E0303), and param types (E0304)
- Is not duplicated (E0306)

Also checks that every call-site use of a generic function whose type
parameter has a trait bound has a valid impl in scope (E0305). The primary
consumer of E0305 is `dbQueryTyped` which requires `impl DbRow for T`.

---

### certo-effects

**Location:** `crates/effects/src/`

Checks that effect annotations (`[io]`, `[async]`, `[db.write]`, `[unsafe]`)
are consistent:
- A function that calls an `[io]` function must itself be `[io]` (E0400/E0401)
- `await` requires `[async]` (E0402)
- `db.transaction` requires `[db.write]` (E0403)
- `unsafe {}` requires `[unsafe]` (E0404)

The pass walks the AST function bodies and propagates effect requirements
upward through the call graph.

---

### certo-dbschema

**Location:** `crates/dbschema/src/`

Validates that `migration` declarations are consistent with `type`
declarations:
- Every `createTable` table must have a corresponding `type` (E0505)
- Column types must match field types (E0501)
- Foreign key targets must be declared types (E0502)
- No duplicate migration names (E0503)
- Non-destructive migrations must have a `down` block (E0504)
- `alterTable`/`dropTable` can only operate on previously created tables (E0507)

Also provides `certo db diff` and `certo db pull` (schema introspection via
`psql`). These are CLI-level features implemented in `crates/cli/src/main.rs`
using `check_module` from this crate to get the expected schema.

---

### certo-hir

**Location:** `crates/hir/src/`

The High-level IR. All surface syntax sugar is desugared here; every node
carries a `Ty` from the type checker.

```
hir/src/
    hir.rs     HirModule, HirFn, HirExpr, HirExprKind, HirStmt, HirPat
    lower.rs   AST → HIR lowering (Cx context)
    error.rs   LowerError
```

**What lowering does:**

| Surface syntax | HIR form |
|---|---|
| `x.method(args)` | `Call { func: Global("Type.method"), args: [x, args] }` |
| `a \|> f` | `Call { func: f, args: [a] }` |
| `x ++ y` | `Call { func: Global("Text.concat"), args: [x, y] }` |
| `x ?? y` | `If { cond: IsNone(x), then_expr: y, else_expr: Unwrap(x) }` |
| `for x in list { body }` | `HirExprKind::For { ... }` |
| `parallel { f(a), g(b) }` | `Block` of `Spawn` + tuple of `Await` |
| `a + b` (Int) | `BinOp { op: Add, ... }` |
| `a + b` (Text) | `Call { func: "Text.concat", ... }` |
| Labeled/default args | Reordered to positional |
| `with` record update | Explicit field-copy construction |

**`Cx` (lowering context)** tracks:
- `locals`: name → `LocalId` stack for lexical scope
- `globals`: function name → `FnId`
- `variant_to_type`: `"Red"` → `"Color"` for constructor desugaring
- `sm_returns`: state machine function return types
- `defers`: accumulated `defer` expressions (emitted LIFO before returns)

**`LocalId`** is a `u32`. Every `HirFn` has its own local numbering starting
at 0. `MirLocal` in the next stage is also a `u32`, and initially `lower_fn`
maps HIR locals to MIR locals with the same indices.

---

### certo-mir

**Location:** `crates/mir/src/`

The Mid-level IR. Functions become explicit control-flow graphs of basic
blocks.

```
mir/src/
    mir.rs     MirFn, BasicBlock, MirStmt, Rvalue, Operand, MirConst, Terminator
    lower.rs   HIR → MIR lowering (Builder)
```

**MirFn structure:**

```
MirFn {
    name:        String
    param_count: usize          // locals[1..=param_count] are params
    locals:      Vec<MirLocalDecl>   // local 0 = return slot
    blocks:      Vec<BasicBlock>     // block 0 = entry
}
```

**BasicBlock:**

```
BasicBlock {
    id:         BlockId
    stmts:      Vec<MirStmt>    // Assign { dest, rvalue }
    terminator: Option<Terminator>
}
```

**Terminators:**

| Variant | Meaning |
|---|---|
| `Goto(bb)` | Unconditional jump |
| `If { cond, true_bb, false_bb }` | Conditional branch |
| `Return(op)` | Function return |
| `Unreachable` | After panic / never-returns |
| `Call { func, args, dest, next }` | Call with result and continuation |
| `Switch { discr, targets, otherwise }` | Match on integer/bool discriminant |

**Rvalue variants:**

| Variant | Meaning |
|---|---|
| `Use(op)` | Copy/move an operand |
| `BinOp { op, lhs, rhs }` | Binary operation |
| `UnOp { op, arg }` | Unary operation |
| `Call { func, args }` | Inline (non-terminal) call |
| `Field { base, field }` | Struct field access |
| `Aggregate(kind, ops)` | Construct tuple/record/array |
| `Spawn { func, args, ret_ty }` | Run `func(args)` on a new OS thread; yields a task handle |
| `Join { task, ret_ty }` | Join a task handle, yielding its result |

**Lambda lifting:** anonymous functions are lifted to top-level `MirFn` items
by the `Builder`. They are named `<enclosing_fn>__lambda_N`. The lifted
functions appear in `Builder.lifted_fns` and are emitted before the
enclosing function in codegen.

**`defer` emission:** deferred expressions are accumulated in
`Builder.defers`. Before every `Return` terminator, the builder emits the
deferred expressions in LIFO (last-in, first-out) order as statements.

---

### certo-codegen

**Location:** `crates/codegen/src/`

The primary backend — emits a single C translation unit.

```
codegen/src/
    emit_module.rs    Entry: emit_module() → String
    emit_mir.rs       emit_fn_with_prefix(), emit_stmt(), emit_operand()
    ty_to_c.rs        Ty → C type string
    emit_validator.rs Validator → Certo source (for validator generator)
```

**`emit_module` order:**

1. System `#include`s and macro guard
2. Runtime header (`RUNTIME_HEADER` — base C types)
3. Stdlib C implementations (injected from `certo-stdlib`)
4. Struct typedefs from `type` declarations (records + tagged unions)
5. State machine struct definitions
6. Forward declarations for all functions
7. Spawn preamble structs + worker functions (for `parallel {}`)
8. Lifted lambda bodies (static)
9. User function bodies

Forward declarations come before all bodies so mutual recursion always
compiles.

**`c_fn_name`:** converts `CamelCase.dotted.name` to `certo_camel_case_dotted_name`
for valid C identifiers. User-defined functions get a `certo_` prefix to
avoid collision with C library names.

**Tagged union layout** for sum types:
```c
typedef enum { Color_Red, Color_Green, Color_Blue } Color_tag_t;
typedef struct {
    Color_tag_t tag;
    union {
        struct { int64_t value; } Red;    // only if variant has fields
    };
} Color;
```

**`RUNTIME_HEADER`:** the string constant that defines `certo_text_t`,
`certo_decimal_t`, `certo_uuid_t`, `CERTO_UNIT`, `CERTO_EXPORT`, and the
pthread shims for `parallel {}`.

---

### certo-stdlib

**Location:** `crates/stdlib/src/`

Each stdlib module is two `const &str` values in its own file:
- `FOO_C` — C implementation of the module's functions
- `FOO_CERTO` — Certo signature declarations for the type-checker

```
stdlib/src/
    lib.rs        full_c_runtime(), full_c_runtime_with_db(), seed_stdlib()
    seed.rs       Registers all stdlib function types into TypeEnv
    core.rs       CORE_C / CORE_CERTO
    collections.rs COLLECTIONS_C / COLLECTIONS_CERTO
    text.rs       TEXT_C / TEXT_CERTO
    datetime.rs   DATETIME_C / DATETIME_CERTO
    money.rs      MONEY_C / MONEY_CERTO
    db.rs         DB_C / DB_CERTO   (libpq-based)
    http.rs       HTTP_C / HTTP_CERTO  (WinHTTP on Windows; stub otherwise)
    json.rs       JSON_C / JSON_CERTO
    math.rs       MATH_C / MATH_CERTO
    crypto.rs     CRYPTO_C / CRYPTO_CERTO
    regex.rs      REGEX_C / REGEX_CERTO
    csv.rs        CSV_C / CSV_CERTO
    env.rs        ENV_C / ENV_CERTO
    file.rs       FILE_C / FILE_CERTO
    path.rs       PATH_C / PATH_CERTO
    process.rs    PROCESS_C / PROCESS_CERTO
```

**`seed_stdlib`** in `seed.rs` calls `TypeEnv::define(name, Ty)` for every
stdlib function. Generic functions use `Ty::Forall` with fresh type variables.
This function is called by the CLI before type-checking every user program.

---

### certo-migrate

**Location:** `crates/migrate/src/`

Reads migration declarations from `.cto` files, maintains a
`.certo_migrations` JSON manifest of which have been applied, and generates
the SQL to apply or roll back steps.

```rust
pub fn plan_up(migrations: &[MigrationDecl], state: &MigrationState)
    -> Vec<MigrationStep>

pub fn plan_down(migrations: &[MigrationDecl], state: &MigrationState, n: usize)
    -> Vec<MigrationStep>

pub fn run_steps(steps: &[MigrationStep], opts: &RunOptions)
    -> Result<Vec<String>, String>
```

Migrations are not executed against a real database by the Rust code —
`run_steps` produces SQL strings. The CLI in `cmd_migrate` calls these and
either prints the SQL (dry-run) or shells out to `psql` to execute it.

---

### certo-fmt

**Location:** `crates/fmt/src/`

The formatter re-parses the source with `certo-parser` and re-emits it from
the AST using its own pretty-printer. This guarantees idempotency.

```rust
pub fn format_source(src: &str) -> Result<String, Vec<ParseError>>
```

---

### certo-diagnostics

**Location:** `crates/diagnostics/src/`

A self-contained diagnostic renderer. No dependency on any other certo crate.

```rust
pub struct Diagnostic {
    pub code:   String,    // "E0200"
    pub message: String,
    pub span:   Option<Span>,
    pub labels: Vec<(Span, String)>,
    pub notes:  Vec<String>,
}

pub fn render_all(
    diags: &[Diagnostic],
    src:   &str,
    filename: &str,
    colour: bool,
) -> String
```

The renderer extracts the relevant source line(s), draws the caret underline,
and applies ANSI colours when `colour` is true (respects `NO_COLOR`).

---

### certo-testrunner

**Location:** `crates/testrunner/src/`

Finds `test` declarations in a parsed module, compiles each one individually
(by generating a C harness that calls the test body), executes it, and
collects pass/fail results with timing.

```rust
pub fn run_file(
    path: &Path,
    opts: &RunOptions,
    colour: bool,
) -> Result<bool, TestRunnerError>
```

---

### certo-llvm

**Location:** `crates/llvm/src/`

An experimental LLVM IR emitter for the MIR. Not used by the main build
pipeline; exists as an alternative backend. Emits textual LLVM IR (`.ll`).

Currently covers basic arithmetic, function calls, `if` / `match`, and
records. `Spawn` and `Join` (concurrency) emit placeholder `i64 0` values — the
C backend is the one with real threading.

---

### certo-lsp

**Location:** `crates/lsp/src/`

A Language Server Protocol server. Communicates over stdin/stdout using the
LSP JSON-RPC protocol. Provides:
- Hover (type of the expression under cursor)
- Diagnostics on save (re-runs the parser and type-checker)
- Go-to definition (within the same file)

---

### certo-cli

**Location:** `crates/cli/src/`

The `certo` binary. Wires together every other crate.

```
cli/src/
    main.rs         Command dispatch, cmd_build, cmd_run, cmd_check,
                    cmd_fmt, cmd_test, cmd_lint, cmd_bench, cmd_new,
                    cmd_migrate, cmd_db, cmd_db_diff, cmd_db_pull
    cmd_doc.rs      certo doc — HTML generator
    cmd_generate.rs certo generate validators — YAML → Certo
    cmd_lint.rs     HIR dataflow pass for L001–L005
    cmd_repl.rs     Interactive REPL
    cmd_watch.rs    File watcher for --watch
```

`type_error_to_diagnostic` in `main.rs` converts every `TypeErrorKind` into
a `Diagnostic` with context-specific hints. This is the function to edit
when improving error messages.

### Other crates

| Crate | Purpose |
|---|---|
| `certo-ffi` | FFI helpers for calling Certo from C |
| `certo-ui` | View / form compiler |
| `certo-wasm` | WebAssembly backend |
| `certo-xeq` | Bytecode interpreter (experimental) |

---

## 5. Key data structures

### `S<T>` — span newtype

```rust
// crates/ast/src/span.rs
pub struct S<T> { pub node: T, pub span: Span }
pub struct Span { pub start: usize, pub end: usize }  // byte offsets
```

Every AST node is wrapped in `S<T>`. Access the inner value with `.node`
and the source location with `.span`.

### `Ty` — the type language

```rust
// crates/typeck/src/ty.rs
pub enum Ty {
    Int | Int8 | Int16 | Int32 | UInt | Float | Decimal | Bool | Text | Unit | Uuid
    Option(Box<Ty>)
    Result(Box<Ty>, Box<Ty>)
    List(Box<Ty>)
    Map(Box<Ty>, Box<Ty>)
    Tuple(Vec<Ty>)
    Named { name: String, args: Vec<Ty> }
    Record(Vec<(String, Ty)>)
    Fn { params: Vec<Ty>, ret: Box<Ty> }
    Var(TyVar)          // unresolved inference variable
    Forall { vars: Vec<TyVar>, body: Box<Ty> }  // polymorphic type
    Error               // inference failed — suppresses cascades
}
```

### `HirExprKind` — desugared expressions

After HIR lowering, the expression tree has no surface sugar. Notable
variants:
- `Local(LocalId)` — reference to a stack variable
- `Global(String)` — reference to a function or constant by name
- `BinOp { op, lhs, rhs }` — primitive arithmetic/comparison
- `Call { func, args }` — all calls (including method calls, pipe)
- `For / While / Spawn / Await` — control flow and concurrency
- `Match { scrutinee, arms }` — pattern match

### `MirFn` — control flow graph

```
blocks[0]        entry block
blocks[N].stmts  sequence of Assign statements
blocks[N].terminator  one of: Goto, If, Return, Unreachable, Call, Switch
```

The `Builder` maintains a `current: BlockId` and appends to
`blocks[current]`. Branching creates new blocks and fills in their
terminators.

---

## 6. How to add a built-in function

Built-in functions that are core language primitives (not library functions)
live in `crates/typeck/src/env.rs` in `TypeEnv::seed_builtins`. All other
functions are stdlib functions (see §7).

**Steps:**

1. **Register the type** in `TypeEnv::seed_builtins` (or in `seed_stdlib` for
   stdlib modules):

   ```rust
   // crates/typeck/src/env.rs
   env.define("myBuiltin", Ty::Fn {
       params: vec![Ty::Int],
       ret: Box::new(Ty::Text),
   });
   ```

2. **Emit a C call** in `crates/codegen/src/emit_mir.rs` inside `emit_rvalue`
   for the `Rvalue::Call` arm — or, for builtins that have a direct C
   function, add the C name mapping in `crates/codegen/src/emit_module.rs`.

   For most builtins, `c_fn_name("myBuiltin")` → `"certo_my_builtin"` is all
   the mapping needed. The C function of that name must exist in the runtime.

3. **Implement the C function** either in `crates/stdlib/src/core.rs` (for
   core builtins) or in a new stdlib module.

4. **Add a test** in `crates/typeck/src/tests.rs` verifying the type is
   inferred correctly, and in `crates/codegen/src/tests.rs` verifying the
   generated C contains the expected call.

---

## 7. How to add a stdlib module

Example: adding `Stdlib.Uuid`.

### Step 1 — Create the C implementation file

```rust
// crates/stdlib/src/uuid.rs

pub const UUID_C: &str = r#"
/* ================================================================
   Stdlib.Uuid
   ================================================================ */
#include <stdlib.h>
#include <string.h>

certo_text_t certo_uuid_v4(void) {
    // ... generate UUID ...
}
"#;

pub const UUID_CERTO: &str = r#"
module Stdlib.Uuid

fn Uuid.v4(): Text [io]
"#;
```

### Step 2 — Register in `crates/stdlib/src/lib.rs`

```rust
mod uuid;
pub use uuid::{UUID_C, UUID_CERTO};

pub fn full_c_runtime() -> String {
    [CORE_C, COLLECTIONS_C, ..., UUID_C].concat()
}

pub fn certo_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ...,
        ("Stdlib.Uuid", UUID_CERTO),
    ]
}
```

### Step 3 — Register types in `crates/stdlib/src/seed.rs`

```rust
// inside seed_stdlib()
def!("Uuid.v4", fn0(Ty::Text));
```

(Add `fn0` helper if it doesn't exist: `Ty::Fn { params: vec![], ret: Box::new(ret) }`.)

### Step 4 — Test

Add `#[test]` in `crates/stdlib/src/tests.rs` and `crates/codegen/src/tests.rs`.

---

## 8. How to add a new keyword / syntax

Example: adding a `repeat n { body }` loop.

### Step 1 — Add the token

```rust
// crates/lexer/src/lib.rs
#[token("repeat")]
Repeat,
```

### Step 2 — Add the AST node

```rust
// crates/ast/src/expr.rs
pub enum Expr {
    ...
    Repeat { count: Box<S<Expr>>, body: Box<S<Expr>> },
}
```

### Step 3 — Parse it

```rust
// crates/parser/src/parse_expr.rs
Token::Repeat => {
    self.advance();
    let count = self.parse_expr()?;
    self.expect(Token::LBrace)?;
    let body = self.parse_block()?;
    self.expect(Token::RBrace)?;
    Ok(S::new(Expr::Repeat { count: Box::new(count), body: Box::new(body) }, span))
}
```

### Step 4 — Lower to HIR

```rust
// crates/hir/src/lower.rs  (inside lower_expr)
Expr::Repeat { count, body } => {
    // desugar to: for _ in range(0, count) { body }
    let count_hir = lower_expr(count, cx);
    let range = HirExpr {
        kind: HirExprKind::Call {
            func: Box::new(global("range")),
            args: vec![int_literal(0), count_hir],
        },
        ty: Ty::List(Box::new(Ty::Int)),
        span: count.span,
    };
    HirExpr {
        kind: HirExprKind::For {
            binding: cx.fresh_local(),
            binding_name: "_".into(),
            binding_ty: Ty::Int,
            iter: Box::new(range),
            body: Box::new(lower_expr(body, cx)),
        },
        ty: Ty::Unit,
        span: expr.span,
    }
}
```

Once desugared to existing HIR nodes, MIR lowering and codegen require no
changes.

### Step 5 — Tests

Add parse tests in `crates/parser/src/tests.rs`, HIR tests in
`crates/hir/src/tests.rs`, and a codegen test in `crates/codegen/src/tests.rs`.

---

## 9. How to add a compiler error

### Step 1 — Add the error kind

Add a variant to the relevant error enum:

```rust
// e.g. crates/typeck/src/error.rs
pub enum TypeErrorKind {
    ...
    /// E0210 — Decimal operation on non-Decimal type.
    DecimalOnNonDecimal { found: Ty },
}
```

### Step 2 — Emit the error at the detection site

```rust
// inside infer_expr.rs
ctx.errors.push(TypeError {
    kind: TypeErrorKind::DecimalOnNonDecimal { found: found_ty.clone() },
    span: expr.span,
});
return Ty::Error;  // suppress cascades
```

### Step 3 — Convert to Diagnostic in the CLI

```rust
// crates/cli/src/main.rs — in type_error_to_diagnostic()
TypeErrorKind::DecimalOnNonDecimal { found } => {
    let names = assign_var_names(&[found]);
    Diagnostic::error("E0210",
        format!("Decimal operation on non-Decimal type `{}`",
                found.display_named(&names)))
        .with_span(e.span)
        .with_note("use Decimal.fromInt(n) to convert an Int to Decimal")
}
```

### Step 4 — Document in `docs/ERROR-REFERENCE.md`

Add an entry under the appropriate range with the error message format,
cause, and fix.

---

## 10. How to add a lint rule

Lint rules live in `crates/cli/src/cmd_lint.rs`. They walk HIR expressions
inside `HirFn` bodies.

**Example: L006 — empty match arm body**

```rust
// crates/cli/src/cmd_lint.rs

// inside lint_fn_body (or a new pass over HirExprKind::Match)
HirExprKind::Match { arms, .. } => {
    for arm in arms {
        if matches!(arm.body.kind, HirExprKind::Unit) {
            emit(path, src, arm.body.span, "L006",
                 "match arm returns Unit — is this intentional?", color);
        }
    }
}
```

Add the code to the help text in `cmd_lint`'s `--help` arm and document it
in `docs/ERROR-REFERENCE.md`.

---

## 11. Testing conventions

### Unit tests

Each crate has `#[cfg(test)] mod tests;` in its `lib.rs`. Tests live in
`crates/<crate>/src/tests.rs`.

**Parser tests** — parse a snippet and check the AST structure:

```rust
#[test]
fn parse_if_else() {
    let m = parse("module A\nfn f(): Int = if true then 1 else 2").unwrap();
    // assert on m.decls[0]
}
```

**Type-checker tests** — assert on error presence/absence and codes:

```rust
#[test]
fn mismatch_int_text() {
    let (_, errs) = check("module A\nfn f(): Int = \"hello\"");
    assert!(!errs.is_empty());
    assert!(errs[0].message().contains("E0200"));
}
```

**Codegen tests** — `codegen(src)` → String, then `assert_contains`:

```rust
#[test]
fn parallel_emits_pthread() {
    let c = codegen("module A\npub fn run(a: Int): Int = {\n\
        val r = parallel { work(a) }\n0\n}");
    assert_contains(&c, "pthread_create");
}
```

### Integration tests

Full build-and-run tests live as `.cto` files compiled and executed by the
test runner. Place them in `tests/` at the workspace root (or in
`crates/testrunner/src/tests.rs`).

### What to test

- Happy path: the feature works
- Error cases: the right error code is emitted
- Edge cases: empty input, single-element, nested
- Codegen: the generated C contains the expected identifiers/patterns

---

## 12. Diagnostics and error formatting

Use `certo-diagnostics` everywhere. Never `eprintln!` directly from a
compiler crate; always build a `Diagnostic` and let the CLI render it.

```rust
use certo_diagnostics::Diagnostic;

let d = Diagnostic::error("E0200", "type mismatch: expected Int, found Text")
    .with_span(span)
    .with_label("this expression has type Text")
    .with_note("use `intToText(n)` to convert an Int to Text");
```

The CLI renders with `render_all(&diags, src, filename, colour)` and prints
the result to stderr.

**Span accuracy:** always attach the smallest sensible span. For a type
mismatch, use the expression's span, not the whole statement's span. This
is what produces the narrow `^^^` underline in error output.

**Notes vs labels:**
- `.with_label(msg)` — appears under the `^^^` underline at the span
- `.with_note(msg)` — appears after the caret block as `= note: ...`

Use labels for "what this specific expression is", notes for "what you should
do about it".

---

## 13. Code style

- **Rust edition 2021**, `resolver = "2"`.
- No `#[allow(dead_code)]` or `#[allow(unused)]` in committed code — fix the
  warning instead.
- Match arms in alphabetical/logical order where the compiler doesn't enforce.
- Error types use snake-case variant names (`TypeErrorKind::ArityMismatch`)
  but are described in documentation as `E0204`.
- No `unwrap()` in library code except where documented as "can't fail because
  ...". Prefer `expect("reason")` or proper error propagation.
- Prefer `writeln!(out, "...")` over `out.push_str(...)` in the codegen
  emitters — the intent is clearer.
- Tests use `assert_contains` / `assert_not_contains` helpers (defined in each
  test module) rather than searching the output with raw `assert!`.
- Keep crate boundaries clean: `certo-codegen` must not depend on
  `certo-parser`; `certo-typeck` must not depend on `certo-hir`. The
  dependency DAG is enforced by Cargo.

### Adding a dependency

Add to the crate's `Cargo.toml`. For workspace-wide dependencies (used in
multiple crates), add to `[workspace.dependencies]` in the root `Cargo.toml`
and reference with `{ workspace = true }` in each crate.

Currently the only workspace dependency is `logos = "0.15"` (used only in
`certo-lexer`). Keep the dependency count low — every new crate added to the
workspace is a cost to compile times for everyone.
