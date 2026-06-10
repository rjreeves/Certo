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

### Option 1 — Download a pre-built binary

Grab the latest release from the [Releases page](https://github.com/rjreeves/Certo/releases):

| Platform | File |
|----------|------|
| Windows x86-64 | `certo-windows-x86_64.zip` |
| Linux x86-64   | `certo-linux-x86_64.tar.gz` |
| macOS ARM64    | `certo-macos-aarch64.tar.gz` |

Extract and put `certo` (or `certo.exe`) somewhere on your `PATH`.

**C compiler required** — Certo compiles via C, so you need `clang`, `gcc`, or `cc` on your PATH.
On Windows, install [LLVM](https://releases.llvm.org/) or use MinGW/MSYS2.

### Option 2 — Install from source with Cargo

```sh
cargo install --git https://github.com/rjreeves/Certo --bin certo
```

Requires Rust 1.75+ (`rustup` recommended). The binary is installed to `~/.cargo/bin/certo`.

### Option 3 — Build manually

```sh
git clone https://github.com/rjreeves/Certo
cd Certo
cargo build --release
# binary is at target/release/certo (or certo.exe on Windows)
```

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
- **Stdlib** — Core, Text, Collections, DateTime, Money, Json, Http, Math, Crypto, Regex, Csv
- **DLL output** with `--emit-dll` for embedding in other apps
- **WASM target** via `certo-wasm`

## Editor support

Certo ships a language server with diagnostics, hover, completion, and go-to-definition.
See [editors/README.md](editors/README.md) for VS Code and Neovim setup instructions.

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
