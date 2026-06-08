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
| 9  | C transpiler backend | `crates/codegen` |
| 10 | Stdlib — Core, Collections, Text, DateTime, Money | `crates/stdlib` |
| 11 | Test runner (`certo-test`) | `crates/testrunner` |
| 12 | Migration tool | `crates/migrate`, `crates/cli` |
| 14 | LSP server | `crates/lsp` |
| 13 | LLVM backend (`certo-llvm`) | `crates/llvm` |
| 15 | Formatter (`certo fmt`) | `crates/fmt` |
| 18 | WASM target (`certo-wasm`) | `crates/wasm` |
| 17 | C FFI and REST client generation | `crates/ffi` |

## Pending

| # | Task | Blocked by | Notes |
|---|------|------------|-------|
| 16 | UI compiler | #9 | Schema-driven view/form generation, Htmx output |

## Critical path to a running program

```
AST → Parser → Resolve → Typeck → HIR → MIR → Codegen (C) → gcc/clang → binary
 1       2        3         4       8     8         9
```

Tasks #1–#15, #17, and #18 are complete. The core pipeline (parse → resolve →
typeck → HIR → MIR → C → binary) is fully operational. Stdlib, test runner,
migration tool, LSP server, formatter, LLVM backend, WASM target, and C FFI /
REST codegen are all in place.

Remaining work: UI compiler (#16) — schema-driven view/form → Htmx output.
