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
8. [Compiling to a Shared Library (DLL)](#8-compiling-to-a-shared-library-dll)
9. [Code Formatting](#9-code-formatting)
10. [LLVM IR & WebAssembly](#10-llvm-ir--webassembly)
11. [FFI — C Headers & REST Clients](#11-ffi--c-headers--rest-clients)
12. [UI Views & Forms](#12-ui-views--forms)
13. [Language Server (LSP)](#13-language-server-lsp)
14. [Building the Compiler from Source](#14-building-the-compiler-from-source)
15. [Release & Distribution Scripts](#15-release--distribution-scripts)
16. [Compiler Pipeline Reference](#16-compiler-pipeline-reference)

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

Create `hello.certo`:

```
module Hello

fn main(): Unit [io] = println("Hello, world!")
```

Compile and run:

```powershell
certo hello.certo -o hello.exe
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

### Let bindings

```
fn hypotenuse(a: Float, b: Float): Float = {
    let a2 = a * a
    let b2 = b * b
    sqrt(a2 + b2)
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

### Recursion

Certo does not have loops — use tail-recursive functions or higher-order
functions from the standard library:

```
fn sumList(xs: List<Int>): Int =
    List.fold(xs, 0, (acc, x) => acc + x)
```

---

## 5. Modules

Each `.certo` file declares exactly one module at the top:

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

Write test blocks in any `.certo` file:

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
certo-test math_test.certo
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

## 8. Compiling to a Shared Library (DLL)

```powershell
certo math.certo --emit-dll -o math.dll
```

This produces:
- `math.dll` — the shared library
- `math.lib` — Windows import library (link against this from C/C++)

All `pub fn` declarations are exported with `CERTO_EXPORT`
(`__declspec(dllexport)` on Windows, `visibility("default")` elsewhere).

Generate a C header for the exported symbols:

```powershell
certo-ffi --header math.certo -o math.h
```

---

## 9. Code Formatting

Format a source file in-place:

```powershell
certo-fmt myfile.certo
```

The formatter is idempotent and enforces the canonical style:
two-space indentation, operators spaced, one blank line between
top-level declarations.

---

## 10. LLVM IR & WebAssembly

### LLVM IR

```powershell
certo-llvm math.certo -o math.ll
certo-llvm math.certo -o math.ll --annotate    # adds type comments
```

You can then compile the IR with:

```powershell
clang math.ll -o math.exe
```

### WebAssembly

```powershell
certo-wasm math.certo --emit-ir -o math.wasm.ll
```

Emits LLVM IR with the `wasm32-unknown-unknown` target triple.
Compile to `.wasm` with:

```bash
clang --target=wasm32 --no-standard-libraries -Wl,--export-all \
      -Wl,--no-entry -o math.wasm math.wasm.ll
```

---

## 11. FFI — C Headers & REST Clients

### C header from a Certo module

```powershell
certo-ffi --header math.certo -o math.h
```

Generates a C header with `extern` declarations for all `pub fn` symbols.
Include it from C to call into a compiled Certo DLL.

### REST client from an OpenAPI schema

```powershell
certo-ffi --rest-client api_schema.json -o TaskApi.certo
```

Reads a JSON schema (`api_schema.json`) and emits a Certo source file
with typed function stubs for every endpoint.

---

## 12. UI Views & Forms

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
certo-ui views.certo -o out\
```

Produces one `.html` file per `view` declaration in `out/`.

---

## 13. Language Server (LSP)

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
require('lspconfig').certo.setup {
  cmd = { 'certo-lsp' },
  filetypes = { 'certo' },
  root_dir = require('lspconfig.util').root_pattern('Cargo.toml', '.git'),
}
```

The LSP provides: hover documentation, go-to-definition, diagnostics,
completion, and formatting.

---

## 14. Building the Compiler from Source

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
examples/      Sample .certo files
scripts/       Build and release automation
```

---

## 15. Release & Distribution Scripts

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

## 16. Compiler Pipeline Reference

```
Source (.certo)
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
certo math.certo --emit-c
```

This is useful for debugging and for understanding how Certo maps to C.

---

## Quick-reference cheat sheet

```
# Compile to executable
certo myapp.certo -o myapp.exe

# Compile to DLL + import lib
certo mylib.certo --emit-dll -o mylib.dll

# Print generated C (no compilation)
certo myapp.certo --emit-c

# Verbose (shows clang invocation)
certo myapp.certo -o myapp.exe -v

# Run unit tests
certo-test myapp_test.certo

# Format source file
certo-fmt myfile.certo

# Generate C header
certo-ffi --header mylib.certo -o mylib.h

# Generate REST client
certo-ffi --rest-client schema.json -o Client.certo

# Emit LLVM IR
certo-llvm mylib.certo -o mylib.ll

# Emit WASM IR
certo-wasm mylib.certo --emit-ir -o mylib.wasm.ll

# Compile UI views to HTML
certo-ui views.certo -o out/

# Build all binaries from source
.\scripts\release.ps1

# Package a release archive
.\scripts\dist-package.ps1 -version "0.1.0"
```
