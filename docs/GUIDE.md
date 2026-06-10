# Certo — Developer Guide

> How to write, build, test, and ship Certo programs.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Installing Certo](#2-installing-certo)
3. [Your First Program](#3-your-first-program)
4. [Language Basics](#4-language-basics)
5. [Modules](#5-modules)
6. [Standard Library](#6-standard-library)
7. [Testing](#7-testing)
8. [Linting, Benchmarking, and the REPL](#8-linting-benchmarking-and-the-repl)
9. [Compiling to a Shared Library (DLL)](#9-compiling-to-a-shared-library-dll)
10. [Code Formatting](#10-code-formatting)
11. [LLVM IR & WebAssembly](#11-llvm-ir--webassembly)
12. [FFI — C Headers & REST Clients](#12-ffi--c-headers--rest-clients)
13. [UI Views & Forms](#13-ui-views--forms)
14. [Language Server (LSP)](#14-language-server-lsp)
15. [Building the Compiler from Source](#15-building-the-compiler-from-source)
16. [Release & Distribution Scripts](#16-release--distribution-scripts)
17. [Compiler Pipeline Reference](#17-compiler-pipeline-reference)

---

## 1. Prerequisites

| Tool | Minimum | Purpose |
|------|---------|---------|
| [Clang / LLVM](https://github.com/llvm/llvm-project/releases) | 15+ | C backend compilation |
| [Rust & Cargo](https://rustup.rs) | 1.77+ | Building the compiler from source |

**Windows quick-install:**
```powershell
winget install LLVM.LLVM
winget install Rustlang.Rustup
```

**macOS:**
```bash
brew install llvm
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Clang must be on `PATH` **or** installed at `C:\Program Files\LLVM\bin\clang.exe`
(the compiler probes both automatically).

---

## 2. Installing Certo

### Option A — pre-built binaries (recommended)

Download a release archive from the releases page and extract it.
The archive contains these executables:

| Binary | Purpose |
|--------|---------|
| `certo` | Main compiler — Certo → native executable or DLL |
| `certo-test` | Test runner |
| `certo-fmt` | Code formatter |
| `certo-lsp` | Language server (for editor integration) |
| `certo-llvm` | Emit LLVM IR |
| `certo-wasm` | Emit WebAssembly IR |
| `certo-ffi` | C header / REST client generator |
| `certo-ui` | Compile `view` / `form` declarations to Htmx HTML |

Add the extracted folder to `PATH`.

### Option B — build from source

```powershell
git clone https://github.com/you/certo
cd certo
.\scripts\release.ps1          # builds & copies all binaries to dist/
```

See [§14](#14-building-the-compiler-from-source) for details.

---

## 3. Your First Program

Create `hello.cto`:

```
module Hello

fn main(): Unit [io] = println("Hello, world!")
```

Compile and run:

```powershell
certo hello.cto -o hello.exe
.\hello.exe
# Hello, world!
```

### Anatomy of a program

- Every file starts with `module <Name>`.
- `fn main(): Unit [io]` is the entry point.  The `[io]` effect annotation
  declares that this function performs I/O.
- `println` is from `Stdlib.Core` and is always in scope.

---

## 4. Language Basics

### Types

| Certo type | C equivalent | Notes |
|------------|-------------|-------|
| `Int` | `int64_t` | 64-bit signed integer |
| `Float` | `double` | 64-bit IEEE 754 |
| `Bool` | `bool` | `true` / `false` |
| `Text` | `char*` | UTF-8 string (heap-allocated) |
| `Unit` | `void` / `int64_t 0` | No meaningful value |
| `T?` | `T*` (nullable) | Option type — `None` is null |
| `List<T>` | length-prefixed heap array | Persistent / copy-on-write |
| `Map<K,V>` | hash map | Key-value store |
| `DateTime` | `struct tm` | Calendar date-time |
| `Decimal` | scaled int64 | Exact decimal arithmetic |

### Functions

```
fn add(a: Int, b: Int): Int = a + b

fn greet(name: Text): Text = "Hello, " ++ name ++ "!"
```

Single-expression functions use `=`.  Block functions use `{ }`:

```
fn describe(n: Int): Unit [io] = {
    print("The number is: ")
    println(intToText(n))
}
```

### `pub` visibility

```
pub fn square(n: Int): Int = n * n   // exported from the module
fn helper(): Int = 42                // private
```

### If / else

```
fn sign(n: Int): Text =
    if n > 0 then "positive"
    else if n < 0 then "negative"
    else "zero"
```

### Val / var bindings

```
fn hypotenuse(a: Float, b: Float): Float = {
    val a2 = a * a
    val b2 = b * b
    sqrt(a2 + b2)
}
```

Use `val` for immutable bindings and `var` for mutable ones:

```
fn countDown(n: Int): Unit = {
    var i = n
    while i > 0 {
        println(intToText(i))
        i = i - 1
    }
}
```

### Operators

| Operator | Meaning |
|----------|---------|
| `+` `-` `*` `/` `%` | Arithmetic (Int or Float) |
| `**` | Integer exponentiation |
| `++` | Text concatenation |
| `==` `!=` `<` `<=` `>` `>=` | Comparison |
| `and` `or` `not` / `!` | Boolean logic |
| `??` | Null coalesce — `a ?? b` returns `a` if non-null, else `b` |
| `?` | Result propagation — `expr?` unwraps `Ok` or returns `Err` early |
| `\|>` | Pipe — `x \|> f` is `f(x)` |
| `..` `...` | Range (inclusive / exclusive) |

### Effects

Effects are declared in square brackets on function signatures.
The compiler tracks them and prevents calling `[io]` functions from
pure contexts.

```
fn readName(): Text [io] = readLine() ?? "anonymous"
```

Common effects: `[io]`, `[db]`, `[fallible]`.

### Loops and iteration

Certo has `for` loops over lists and `while` loops:

```
fn printAll(items: List<Text>): Unit = {
    for item in items {
        println(item)
    }
}

fn countdown(n: Int): Unit = {
    var i = n
    while i > 0 {
        println(intToText(i))
        i = i - 1
    }
}
```

Higher-order functions are also available for functional style:

```
fn sumList(xs: List<Int>): Int =
    List.fold(xs, 0, (acc, x) => acc + x)
```

### Error handling with Result and ?

Functions that can fail return `Result<T, E>`. Use `Ok` and `Err` to construct
results, and `?` to propagate errors early:

```
fn divide(a: Int, b: Int): Result<Int, Text> =
    if b == 0 then Err("division by zero")
    else Ok(a / b)

fn compute(a: Int, b: Int): Result<Int, Text> = {
    val x = divide(a, b)?   // returns Err early if divide fails
    Ok(x * 2)
}
```

### Guard clauses

`guard` validates a condition and returns early if it fails — useful for
precondition checks at the top of a function:

```
fn processRefund(amount: Int, max: Int): Result<Int, Text> = {
    guard amount > 0 else Err("amount must be positive")
    guard amount <= max else Err("amount exceeds maximum")
    Ok(amount)
}
```

---

## 5. Modules

Each `.cto` file declares exactly one module at the top:

```
module MyApp.Utils
```

Nested module names use `.` as a separator.  The module name is
independent of the file path — it is purely a namespace.

### Importing

```
module MyApp

import Stdlib.Text
import MyApp.Utils
```

All names from `Stdlib.Core` are available automatically without an
explicit import.

---

## 6. Standard Library

All standard library functions are in scope once imported.
`Stdlib.Core` is always available without an import.

### Stdlib.Core

```
// Output
println(s: Text): Unit [io]
print(s: Text): Unit [io]
eprintln(s: Text): Unit [io]

// Conversions
intToText(n: Int): Text
floatToText(f: Float): Text
boolToText(b: Bool): Text
floatToInt(f: Float): Int
intToFloat(n: Int): Float
parseInt(s: Text): Int?
parseFloat(s: Text): Float?

// Arithmetic helpers
pow(base: Int, exp: Int): Int
absInt(n: Int): Int          absFloat(f: Float): Float
minInt(a: Int, b: Int): Int  maxInt(a: Int, b: Int): Int
floor(f: Float): Float       ceil(f: Float): Float
round(f: Float): Float       sqrt(f: Float): Float

// Ranges
range(start: Int, end: Int): List<Int>         // [start, end)
rangeInclusive(start: Int, end: Int): List<Int>

// I/O
readLine(): Text? [io]
readAll(): Text [io]
argCount(): Int
arg(i: Int): Text?

// Assertions
assert(cond: Bool, msg: Text): Unit
```

### Stdlib.Text

```
Text.len(s)           Text.concat(a, b)
Text.contains(s, sub) Text.startsWith(s, prefix)
Text.endsWith(s, suf) Text.toUpper(s) / Text.toLower(s)
Text.trim(s)          Text.trimStart(s) / Text.trimEnd(s)
Text.slice(s, i, j)   Text.indexOf(s, sub): Int?
Text.replace(s, from, to)
Text.split(s, sep): List<Text>
Text.join(parts, sep): Text
Text.repeat(s, n): Text
```

### Stdlib.Collections

```
// List
List.empty<T>(): List<T>
List.len(list)         List.get(list, i): T?
List.push(list, item)  List.concat(a, b)
List.first(list): T?   List.last(list): T?
List.slice(list, i, j) List.reverse(list)
List.map(list, f)      List.filter(list, pred)
List.fold(list, init, f)
List.contains(list, item): Bool

// Map
Map.empty<K,V>(): Map<K,V>
Map.insert(map, key, value)
Map.get(map, key): V?
Map.contains(map, key): Bool
Map.remove(map, key)
Map.len(map): Int
Map.keys(map): List<K>
Map.values(map): List<V>
```

### Stdlib.DateTime

```
DateTime.now(): DateTime [io]
Date.today(): Date [io]
DateTime.fromUnix(secs: Int): DateTime
DateTime.toUnix(dt): Int
DateTime.format(dt, fmt: Text): Text   // strftime format
DateTime.toIso(dt): Text               // ISO 8601
DateTime.parseIso(s): DateTime [fallible]
DateTime.addDays(dt, d) / addHours / addMinutes / addSeconds
DateTime.diffDays(a, b): Int
DateTime.before(a, b): Bool  DateTime.after(a, b): Bool
DateTime.year(dt) / month / day / hour / minute / second
```

### Stdlib.Money

```
Decimal.add(a, b)  Decimal.sub(a, b)
Decimal.mul(a, b)  Decimal.div(a, b) [fallible]
Decimal.round(d, places)
Decimal.toText(d): Text
Money.fromCents(cents: Int): Decimal
Money.toCents(m): Int
```

---

## 7. Testing

Write test blocks in any `.cto` file:

```
module MathTest

fn factorial(n: Int): Int = if n <= 1 then 1 else n * factorial(n - 1)

test "factorial(5) = 120" {
    {
        assert(factorial(5) == 120, "factorial(5)")
    }
}

test "5 is prime" {
    {
        assert(isPrime(5), "isPrime(5)")
    }
}
```

Run all tests:

```powershell
certo-test math_test.cto
```

Output:

```
PASS  factorial(5) = 120
PASS  5 is prime
2 passed, 0 failed
```

`assert(cond, message)` aborts with the message if `cond` is false — the
test runner captures the non-zero exit and marks it as `FAIL`.

### dbTest and property

```
dbTest "user exists after insert" {
    { /* runs with a real DB connection */ }
}

property "reverse twice is identity" {
    { /* property-based test */ }
}
```

---

## 8. Linting, Benchmarking, and the REPL

### `certo lint` — static analysis

`certo lint` runs a dataflow pass over the HIR and reports:

- **L001** unused parameter — parameter never read in the function body
- **L002** unused variable — `val`/`var` declared but never read
- **L003** assigned but never read — variable written but the value discarded before the next write
- **L004** unreachable statement — code after a call to `panic`/`todo`/`unreachable`

```powershell
certo lint myapp.cto
```

Names prefixed with `_` suppress all lint warnings for that binding.

### `certo bench` — micro-benchmarks

Declare functions named `bench_*` with no parameters; `certo bench` compiles and
runs each one N times (default 1000) and reports the median time in ns/iter:

```
module MyBench

fn bench_concat(): Text =
    "Hello" ++ ", " ++ "world!"
```

```powershell
certo bench myapp.cto
# bench_concat   42 ns/iter
```

### `certo repl` — interactive session

`certo repl` starts an interactive session. Expressions are evaluated and their
values printed automatically. Declarations (`fn`, `val`, `type`) accumulate in
the session.

```
Certo v1.0.2.260610.52 (c) SyntrA 2026
> 1 + 1
2 : Int
> val name = "Alice"
name
> f"Hello, {name}!"
"Hello, Alice!" : Text
> :help      -- show meta-commands
> :quit
```

---

## 10. Compiling to a Shared Library (DLL)

```powershell
certo math.cto --emit-dll -o math.dll
```

This produces:
- `math.dll` — the shared library
- `math.lib` — Windows import library (link against this from C/C++)

All `pub fn` declarations are exported with `CERTO_EXPORT`
(`__declspec(dllexport)` on Windows, `visibility("default")` elsewhere).

Generate a C header for the exported symbols:

```powershell
certo-ffi --header math.cto -o math.h
```

---

## 10. Code Formatting

Format a source file in-place:

```powershell
certo-fmt myfile.cto
```

The formatter is idempotent and enforces the canonical style:
two-space indentation, operators spaced, one blank line between
top-level declarations.

---

## 11. LLVM IR & WebAssembly

### LLVM IR

```powershell
certo-llvm math.cto -o math.ll
certo-llvm math.cto -o math.ll --annotate    # adds type comments
```

You can then compile the IR with:

```powershell
clang math.ll -o math.exe
```

### WebAssembly

```powershell
certo-wasm math.cto --emit-ir -o math.wasm.ll
```

Emits LLVM IR with the `wasm32-unknown-unknown` target triple.
Compile to `.wasm` with:

```bash
clang --target=wasm32 --no-standard-libraries -Wl,--export-all \
      -Wl,--no-entry -o math.wasm math.wasm.ll
```

---

## 12. FFI — C Headers & REST Clients

### C header from a Certo module

```powershell
certo-ffi --header math.cto -o math.h
```

Generates a C header with `extern` declarations for all `pub fn` symbols.
Include it from C to call into a compiled Certo DLL.

### REST client from an OpenAPI schema

```powershell
certo-ffi --rest-client api_schema.json -o TaskApi.cto
```

Reads a JSON schema (`api_schema.json`) and emits a Certo source file
with typed function stubs for every endpoint.

---

## 13. UI Views & Forms

`view` declarations describe server-rendered UI pages.
`form` declarations describe HTML forms bound to a module endpoint.

```
module TasksUI

view Dashboard {
    layout = VStack(children: [
        Heading("Task Dashboard"),
        Button("New Task", "/tasks/new"),
    ])
}

form NewTask -> Tasks {
    title: Text
    dueDate: DateTime
    onSubmit: submitTask
}
```

Compile to Htmx HTML:

```powershell
certo-ui views.cto -o out\
```

Produces one `.html` file per `view` declaration in `out/`.

---

## 14. Language Server (LSP)

`certo-lsp` implements the Language Server Protocol.

### VS Code

Add to `.vscode/settings.json`:

```json
{
  "certo.languageServerPath": "C:/path/to/dist/certo-lsp.exe"
}
```

### Neovim (nvim-lspconfig)

```lua
require('lspconfig').cto.setup {
  cmd = { 'certo-lsp' },
  filetypes = { 'certo' },
  root_dir = require('lspconfig.util').root_pattern('Cargo.toml', '.git'),
}
```

The LSP provides: hover documentation, go-to-definition, diagnostics,
completion (including position-aware local variable completions), and formatting.

---

## 15. Building the Compiler from Source

### Clone and build

```powershell
git clone https://github.com/you/certo
cd certo
cargo build --release --bins
```

Binaries land in `target\release\`.

### Copy to dist/

```powershell
.\scripts\release.ps1
```

This runs `cargo build --release --bins` and copies the 8 executables
to `dist\`.  See [§15](#15-release--distribution-scripts).

### Running tests

```powershell
cargo test --workspace
```

### Workspace layout

```
crates/
  ast/         AST data types
  lexer/       Tokeniser (Logos)
  parser/      Recursive-descent parser
  resolve/     Name resolution
  typeck/      Type inference (Algorithm W / HM)
  traits/      Trait system
  effects/     Effect type checker
  dbschema/    DB schema validation
  hir/         High-level IR + lowering
  mir/         Mid-level IR
  codegen/     C code generator
  stdlib/      Standard library (C + Certo declarations)
  testrunner/  certo-test harness
  llvm/        LLVM IR emitter
  wasm/        WebAssembly target
  ffi/         C header + REST client generator
  fmt/         Code formatter
  lsp/         Language server
  ui/          View/form → Htmx HTML compiler
  cli/         certo main binary
dist/          Pre-built release binaries
docs/          This guide and the language specification
examples/      Sample .cto files
scripts/       Build and release automation
```

---

## 16. Release & Distribution Scripts

All scripts live in `scripts\` and are run from the **repo root**.

### `scripts\release.ps1` — build & stage

```powershell
.\scripts\release.ps1
```

Runs `cargo build --release --bins`, then copies all 8 binaries to
`dist\`.  Pass `-verbose` to see each copy operation.

### `scripts\dist-package.ps1` — create a release archive

```powershell
.\scripts\dist-package.ps1 -version "0.1.0"
```

Creates `dist\certo-0.1.0-windows-x64.zip` (Windows) or
`certo-0.1.0-linux-x64.tar.gz` (Linux/macOS) containing:

- All 8 executables from `dist\`
- `docs\GUIDE.md`
- `examples\` (source files only, no `out\`)
- `README.md` (if present)

Pass `-out <dir>` to change the output directory.

---

## 17. Compiler Pipeline Reference

```
Source (.cto)
    │
    ▼  crates/lexer      Tokenise
    │
    ▼  crates/parser     Parse → AST
    │
    ▼  crates/resolve    Name resolution
    │
    ▼  crates/typeck     Type inference (HM / Algorithm W)
    │
    ▼  crates/traits     Trait conformance checking
    │
    ▼  crates/effects    Effect tracking
    │
    ▼  crates/hir        Lower AST → HIR
    │
    ▼  crates/mir        Lower HIR → MIR (3-address code)
    │
    ▼  crates/codegen    Emit C source
    │
    ▼  clang             Compile C → native binary / DLL
```

The `--emit-c` flag stops the pipeline after codegen and prints the
generated C to stdout:

```powershell
certo math.cto --emit-c
```

This is useful for debugging and for understanding how Certo maps to C.

---

## Quick-reference cheat sheet

```
# Scaffold a new project
certo new my-project

# Compile to executable
certo myapp.cto -o myapp.exe

# Compile and run immediately
certo run myapp.cto

# Compile to DLL + import lib
certo mylib.cto --emit-dll -o mylib.dll

# Type-check only (no compilation)
certo check myapp.cto

# Print generated C (no compilation)
certo myapp.cto --emit-c

# Verbose (shows clang invocation)
certo myapp.cto -o myapp.exe -v

# Interactive REPL
certo repl

# Run unit tests
certo-test myapp_test.cto

# Lint for unused variables and dead code
certo lint myapp.cto

# Run bench_ functions and report ns/iter
certo bench myapp.cto

# Format source file
certo-fmt myfile.cto

# Generate C header
certo-ffi --header mylib.cto -o mylib.h

# Generate REST client
certo-ffi --rest-client schema.json -o Client.cto

# Emit LLVM IR
certo-llvm mylib.cto -o mylib.ll

# Emit WASM IR
certo-wasm mylib.cto --emit-ir -o mylib.wasm.ll

# Compile UI views to HTML
certo-ui views.cto -o out/

# Build all binaries from source
.\scripts\release.ps1

# Package a release archive
.\scripts\dist-package.ps1 -version "0.1.0"
```
