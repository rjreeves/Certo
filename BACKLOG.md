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

## Pending

| # | Task | Blocked by | Notes |
|---|------|------------|-------|
| 10 | Stdlib — Core, Collections, Text, DateTime, Money | #9 | Certo source + C runtime impls for built-in types |
| 11 | Test runner | #9 | Compile and execute `test "…" { }` and `dbTest "…" { }` blocks |
| 12 | Migration tool | — | CLI: `certo migrate up/down`, status tracking, dry-run |
| 13 | LLVM backend | #9 | Optional high-performance target via LLVM IR emission |
| 14 | LSP server | — | Hover, go-to-definition, diagnostics over stdio/JSON-RPC |
| 15 | Formatter (`certo fmt`) | — | Opinionated AST-based formatter |
| 16 | UI compiler | #9 | Schema-driven view/form generation, Htmx output |
| 17 | C FFI and REST client generation | #9 | `extern` declarations → C headers; REST schema → typed clients |
| 18 | WASM target | #13 | Compile to WebAssembly via LLVM wasm32 target |

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → gcc/clang → binary
 1       2        3         4       8     8         9
```

Tasks #10 (stdlib) and #11 (test runner) are the next steps to get real programs
compiling and running end-to-end.
