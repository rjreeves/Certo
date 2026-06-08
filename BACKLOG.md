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

## Pending

| # | Task | Notes |
|---|------|-------|
| 26 | `for` loops / `forEach` syntax | Iteration without explicit recursion; most-wanted language gap |
| 27 | `match` expressions end-to-end | AST exists; verify HIR → MIR → codegen path works completely |
| 28 | String interpolation `"Hello, {name}!"` | Parser + codegen; desugars to `++` concat calls |
| 29 | `let` destructuring | `let (a, b) = pair`, `let { x, y } = record` |
| 30 | Stdlib-aware name resolver | Pre-load stdlib names so resolve pass can be enabled in the CLI |
| 31 | Resolve + typeck errors in CLI | Blocked on #30; error types and renderer already ready |
| 32 | README at repo root | Short README with code sample, install badge, 60-second getting-started |
| 33 | GitHub Actions CI | `cargo test --workspace` + `scripts/release.ps1` on push |
| 34 | `import` resolver (multi-file projects) | Read `.certo` files from disk; needed for real projects |

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → gcc/clang → binary
 1       2        3         4       8     8         9
```

The core pipeline (parse → HIR → MIR → C → binary) is fully operational.
Resolve and typeck run but are not yet gated in the CLI build path (stdlib
names not yet pre-loaded — task #30).
