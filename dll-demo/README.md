# dll-demo

A minimal `add(a, b)` shared library demonstrating DLL interop in both directions.

## Direction 1 — Certo *exports* a DLL, others consume it

```powershell
certo build add.cto --emit-dll -o add.dll   # -> add.dll + add.lib
certo-ffi --header add.cto -o add.h          # -> add.h (C declarations)
```

`pub fn add(a: Int, b: Int): Int` is exported to native code as the C symbol
**`certo_add`** (`certo_` prefix + snake_case). `Int` is 64-bit (`int64_t`).

| Consumer | File | How it uses the DLL |
|----------|------|---------------------|
| Certo | `caller.cto`     | `import Add` — source-level; recompiles the module, does **not** link the binary |
| C     | `use_dll.c`      | `#include "add.h"`, links `add.lib`, loads `add.dll` |
| C#    | `csharp-demo/`   | `[DllImport("add.dll", EntryPoint = "certo_add")]` P/Invoke |
| Rust  | `rust-demo/`     | `extern "C"` + `#[link(name = "add")]`, links `add.lib` |

```powershell
certo run caller.cto                                   # add(2, 3) = 5
clang use_dll.c add.lib -o use_dll.exe; ./use_dll.exe  # certo_add(2, 3) = 5
cd csharp-demo; dotnet run -c Release                  # certo_add(2, 3) = 5
cd rust-demo; rustc use_dll.rs -L .. -o use_dll_rs.exe; ./use_dll_rs.exe  # certo_add(2, 3) = 5
```

## Direction 2 — Certo *calls into* a Rust DLL  (`extern "C"` FFI)

`call_rust.cto` declares the foreign function and calls it inside an `unsafe { }`
block (FFI calls must be marked unsafe — the compiler cannot verify them; E0210):

```
extern "C" {
    fn rustAdd(a: Int, b: Int): Int
}

fn main(): Unit = {
    val result = unsafe { rustAdd(2, 3) }
    println(f"rustAdd(2, 3) = {result}")
}
```

Certo mangles `rustAdd` to the C symbol `certo_rust_add`, so the Rust side
(`rust-dll/rustmath.rs`) exports that exact name:

```rust
#[no_mangle]
pub extern "C" fn certo_rust_add(a: i64, b: i64) -> i64 { a + b }
```

```powershell
cd rust-dll; rustc --crate-type cdylib rustmath.rs -o rustmath.dll   # -> rustmath.dll + rustmath.dll.lib
certo build call_rust.cto --link rust-dll/rustmath.dll.lib -o rust-dll/call_rust.exe
cd rust-dll; ./call_rust.exe                                          # rustAdd(2, 3) = 5
```

The `--link <lib>` flag and the `extern "C"` block are the FFI feature added to the
compiler for this (spec §12.1). Certo emits a C prototype for the foreign function
and leaves the definition to be resolved by the linked library.
