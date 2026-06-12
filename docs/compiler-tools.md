# Certo Compiler Tools

Eight binaries ship in `dist\`. They all operate on `.cto` source files and are independent — pick the one you need.

---

## `certo` — Main Compiler & Toolchain

The primary tool. Compiles `.cto` files to native executables and exposes subcommands for the full development workflow.

```
certo <file.cto>                   Build (shorthand — no subcommand needed)
certo build <file.cto> [options]   Build explicitly

  -o <file>        Output path (default: <stem>.exe / <stem>)
  --emit-c         Write intermediate C source instead of compiling it
  --emit-dll       Build a shared library (.dll / .so) instead of an exe

certo check <file.cto>             Type-check without producing a binary
certo run   <file.cto> [args...]   Build and immediately run
certo doc   <file.cto>             Generate HTML documentation
certo lint  <file.cto>             Static analysis and style warnings
certo repl                         Interactive REPL (read-eval-print loop)
certo test  <file.cto>             Run test blocks (same as certo-test)
certo db    <subcommand>           Database tools (see below)
certo migrate <subcommand>         Alias for certo db
```

**DB subcommands** (`certo db …`):

| Subcommand | Description |
|------------|-------------|
| `migrate`  | Apply pending migrations |
| `rollback` | Roll back the last migration |
| `status`   | Show which migrations have been applied |
| `pull`     | Introspect a live database and generate a schema |

**Build examples:**

```powershell
certo build examples\msgbox.cto
certo build examples\msgbox.cto -o dist\msgbox.exe
certo build mylib.cto --emit-dll
certo run   examples\generic_test.cto
```

---

## `certo-test` — Test Runner

Discovers and runs `test` blocks inside `.cto` files. Exit code 0 = all pass, 1 = failures, 2 = compile error.

```
certo-test [options] <file.cto>...

  --filter <pattern>   Only run tests whose name contains <pattern>
  --no-color           Disable ANSI colour output
```

```powershell
certo-test examples\math_test.cto
certo-test --filter "factorial" examples\math_test.cto
```

Test blocks in source look like:

```
test "factorial of 5" {
    assert factorial(5) == 120
}
```

---

## `certo-lsp` — Language Server

Implements the [Language Server Protocol](https://microsoft.github.io/language-server-protocol/) for editor integration. Runs as a background process started by your editor — you do not invoke it directly.

**Features:** diagnostics, hover types, go-to-definition, completions.

**VS Code:** point the Certo extension at `dist\certo-lsp.exe` in settings.  
**Any LSP-capable editor:** configure the binary path and use `stdio` transport.

---

## `certo-fmt` — Code Formatter

Formats `.cto` source files in place (or checks formatting in CI).

```
certo-fmt <file.cto>...     Format files in place
certo-fmt --check <file.cto>...   Exit non-zero if any file would change
certo-fmt                         Read from stdin, write to stdout
```

```powershell
certo-fmt examples\msgbox.cto
certo-fmt --check examples\*.cto   # CI gate
Get-Content myfile.cto | certo-fmt | Set-Content myfile_formatted.cto
```

---

## `certo-ffi` — C Header & REST Client Generator

Generates interop artifacts from Certo source:

- **C header** — exposes `pub` functions so C/C++ code can call Certo DLLs.
- **REST client stubs** — generates typed Certo client code from a JSON REST schema.

```
certo-ffi --header     <file.cto>      [-o <out.h>]   [--guard <GUARD_H>]
certo-ffi --rest-client <schema.json>  [-o <out.cto>]
```

```powershell
# Build the DLL, then generate a matching C header
certo build mylib.cto --emit-dll
certo-ffi --header mylib.cto -o mylib.h

# Generate Certo client stubs from a REST API schema
certo-ffi --rest-client api-schema.json -o client.cto
```

---

## `certo-llvm` — LLVM IR Emitter

Compiles a `.cto` file to LLVM IR (`.ll` text format) instead of going all the way to a binary. Useful for inspecting code-gen output, targeting non-C backends, or cross-compilation.

```
certo-llvm [options] <file.cto>

  -o <file>                Output file (default: <stem>.ll; use - for stdout)
  --target <triple>        LLVM target triple (e.g. x86_64-unknown-linux-gnu)
  --data-layout <layout>   LLVM data layout string
  --wasm                   Shorthand for --target wasm32-wasi
  --annotate               Emit comment annotations in the IR
```

```powershell
certo-llvm examples\math.cto               # produces math.ll
certo-llvm --wasm examples\math.cto        # WASM IR
certo-llvm -o - examples\math.cto         # dump to stdout
```

---

## `certo-wasm` — WebAssembly Compiler

Compiles Certo to `.wasm`. Targets either WASI (server-side / CLI runtimes like Wasmtime) or browser (`wasm32-unknown-unknown`). Optionally generates TypeScript bindings and an HTML demo page.

```
certo-wasm [options] <file.cto>

  -o <file>              Output path (.wasm default)
  --target wasi          wasm32-wasi  (default)
  --target browser       wasm32-unknown-unknown
  --wasi-sysroot <path>  Path to WASI sysroot (auto-detected if omitted)
  --opt <0-3>            Optimisation level (default: 2)
  --emit-ir              Emit LLVM IR (.ll) instead of .wasm
  --bindings             Generate .d.ts and .js bindings alongside .wasm
  --html                 Also generate a browser demo .html file
  --no-color             Disable ANSI colour output
```

```powershell
certo-wasm examples\math.cto                        # math.wasm (WASI)
certo-wasm --target browser --bindings examples\math.cto  # math.wasm + math.d.ts + math.js
certo-wasm --html examples\math.cto                 # + math.html demo page
```

---

## `certo-ui` — View/Form Compiler

Compiles Certo `view` and `form` declarations to [Htmx](https://htmx.org/)-powered HTML files. Used for server-rendered UI without writing raw HTML.

```
certo-ui [options] <file.cto>

  -o <dir>     Output directory (default: current directory)
  --stdout     Print generated files to stdout instead of writing
  --no-color   Disable ANSI colour in status messages
```

```powershell
certo-ui examples\views.cto -o dist\html
certo-ui --stdout examples\views.cto
```

Source example:

```
view UserCard(user: User) = {
    div {
        h2 { user.name }
        p  { user.email }
    }
}
```

---

## Quick Reference

| Binary | Purpose | Typical use |
|--------|---------|-------------|
| `certo` | Build, run, check, REPL, DB migrations | Daily driver |
| `certo-test` | Run `test` blocks | CI / TDD |
| `certo-lsp` | Editor language server | IDE integration |
| `certo-fmt` | Auto-format source | Pre-commit hook |
| `certo-ffi` | Generate C headers / REST clients | Interop |
| `certo-llvm` | Emit LLVM IR | Debugging codegen / cross-compile |
| `certo-wasm` | Compile to WebAssembly | Browser / WASI targets |
| `certo-ui` | Compile views to Htmx HTML | Server-rendered UI |

All binaries live in `dist\` after running `.\build.ps1`.
