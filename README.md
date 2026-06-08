# Certo

A statically typed, compiled programming language designed for safety, clarity, and performance.
Certo compiles to C (via an intermediate MIR) and runs anywhere a C compiler does.

```
module Hello

fn greet(name: Text): Text = f"Hello, {name}!"

fn main(): Unit = {
    val message = greet("world")
    print(message)
}
```

## Install

**Prerequisites:** Rust 1.75+ and a C compiler (clang or gcc).

```powershell
git clone https://github.com/your-org/certo
cd certo
cargo build --release
```

Binaries land in `target/release/`:

| Binary | Purpose |
|--------|---------|
| `certo` | Compiler and runner |
| `certo-test` | Test runner |
| `certo-fmt` | Formatter |
| `certo-lsp` | Language server (VS Code / JetBrains) |

## 60-second getting started

```certo
module Counter

fn main(): Unit = {
    val items = [1, 2, 3, 4, 5]
    var total = 0
    for n in items {
        total = total + n
    }
    print(f"Sum: {total}")
}
```

Save as `counter.certo`, then:

```powershell
certo run counter.certo
```

## Language features

- **Static types** with Hindley-Milner inference
- **Pattern matching** with exhaustiveness checking
- **String interpolation** via `f"Hello, {name}!"`
- **For loops** over lists
- **`++` string concat**, `??` null-coalesce
- **Records, tuples, sum types**
- **Effect types** — async, DB, IO tracked at compile time
- **Stdlib** — Core, Text, Collections, DateTime, Money
- **DLL output** with `--emit-dll` for embedding in other apps
- **WASM target** via `certo-wasm`

## Documentation

See [docs/GUIDE.md](docs/GUIDE.md) for the full developer guide, including:
- Full language syntax reference
- Stdlib API
- Building and testing
- LLVM / WASM targets
- FFI and C interop

## Build scripts

```powershell
scripts/release.ps1          # build release binaries → dist/
scripts/dist-package.ps1     # create zip/tar.gz distribution package
```

## License

MIT
