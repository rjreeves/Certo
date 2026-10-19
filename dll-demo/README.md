# dll-demo

Runnable examples of native interop in both directions: other languages calling
Certo through a shared library, and Certo calling native libraries through
`extern "C"`. The guide that explains the rules is
[`docs/DLL-INTEROP.md`](../docs/DLL-INTEROP.md); this folder is the working code
behind it.

Every command below was run and produces the output shown. Examples use Windows
names (`.dll`, `.lib`, PowerShell).

**You need:** `certo` and `certo-ffi` (from `dist/`), `clang`, and — for the
folders that use them — `rustc` and the .NET SDK. Run commands from inside this
folder.

## What is here

| Folder / file | What it shows |
|---|---|
| `add.cto`, `add.h`, `use_dll.c`, `caller.cto`, `csharp-demo/`, `rust-demo/` | **Direction 1** — a Certo library called from C, C#, Rust and Certo |
| `call_rust.cto`, `rust-dll/` | **Direction 2** — Certo calling a Rust DLL |
| `reverse-e2e/` | Both at once: C# → Certo DLL → Rust DLL, passing `Text` and a record |
| `dataprovider/` | Returning a whole result set to a host as one JSON `Text` |

## Direction 1 — Certo exports a DLL

```powershell
certo build add.cto --emit-dll -o add.dll   # -> add.dll + add.lib
certo-ffi --header add.cto -o add.h          # -> add.h (C declarations)
```

`pub fn add(a: Int, b: Int): Int` is exported as the C symbol **`certo_add`**
(`certo_` + snake_case). `Int` is 64-bit (`int64_t`). Take the name from `add.h`
rather than working it out by hand.

| Consumer | File | How it uses the DLL |
|---|---|---|
| Certo | `caller.cto` | `import Add` — recompiles the module's source; does **not** link the DLL |
| C | `use_dll.c` | `#include "add.h"`, links `add.lib`, loads `add.dll` |
| C# | `csharp-demo/` | `[DllImport("add.dll", EntryPoint = "certo_add")]` P/Invoke |
| Rust | `rust-demo/` | `extern "C"` + `#[link(name = "add")]`, links `add.lib` |

```powershell
certo run caller.cto                                   # add(2, 3) = 5
clang use_dll.c add.lib -o use_dll.exe; ./use_dll.exe  # certo_add(2, 3) = 5
cd csharp-demo; dotnet run -c Release                  # certo_add(2, 3) = 5
cd rust-demo; rustc use_dll.rs -L .. -o use_dll_rs.exe; ./use_dll_rs.exe  # certo_add(2, 3) = 5
```

(`use_dll_rs.exe` needs `add.dll` beside it — copy it into `rust-demo/`.)

## Direction 2 — Certo calls into a Rust DLL

`call_rust.cto` declares the foreign function and calls it inside `unsafe { }`:

```certo
extern "C" {
    fn rustAdd(a: Int, b: Int): Int
}

fn main(): Unit [io, unsafe] = {
    val result = unsafe { rustAdd(2, 3) }
    println(f"rustAdd(2, 3) = {result}")
}
```

Three rules: the call must be inside `unsafe { }` (E0210); the enclosing function
must declare the `[unsafe]` effect (E0404), plus `io` here because it prints; and
**an `extern` name is used verbatim as the C symbol** — `rustAdd` links against
`rustAdd`. So the Rust side exports exactly that name (`rust-dll/rustmath.rs`):

```rust
#[no_mangle]
#[allow(non_snake_case)]
pub extern "C" fn rustAdd(a: i64, b: i64) -> i64 { a + b }
```

```powershell
cd rust-dll; rustc --crate-type cdylib rustmath.rs -o rustmath.dll   # -> rustmath.dll + rustmath.dll.lib
certo build ../call_rust.cto --link rustmath.dll.lib -o call_rust.exe
./call_rust.exe                                                       # rustAdd(2, 3) = 5
```

> Earlier versions of Certo mangled extern names to `certo_rust_add`, and this demo
> used to export that. If you see `undefined symbol: rustAdd`, the native library
> still exports the old mangled name.

## `reverse-e2e/` — all three languages

`strutil.cto` is a Certo library that calls a Rust function to reverse a string
and returns a record `Reversed { reversed: Text, length: Int }`. C# calls it.

```
C# (csharp/Program.cs) ──► strutil.dll (Certo) ──► strrev.dll (Rust, rust-dll/strrev.rs)
```

```powershell
cd reverse-e2e/rust-dll; rustc --crate-type cdylib strrev.rs -o strrev.dll
cd ..; certo build strutil.cto --emit-dll --link rust-dll/strrev.dll.lib -o strutil.dll
cd csharp; dotnet run -c Release
# input:    abcdef
# reversed: fedcba
# length:   6
```

The record crosses the boundary **by value** as `struct { const char* reversed;
int64_t length; }`; `csharp/Program.cs` declares the matching
`[StructLayout(LayoutKind.Sequential)]` struct.

The other files here are the steps the example was built up through, kept as
smaller test programs:

| File | What it checks |
|---|---|
| `min.cto` | the smallest call into the Rust DLL |
| `a.cto` | `Text` across the boundary, and `Text.len` of the result |
| `b.cto` | the result stored in a record |
| `smoketest.cto` | calling the `strutil` library from Certo |
| `f.cto` | a plain record with an f-string, no FFI |
| `c.cto`, `d.cto`, `e.cto` | the same helper-function program printing its result three ways: `println`, an f-string over a `Text` field, an f-string over an `Int` field |
| `f.c`, `min.c` | generated C kept for comparison — not needed to run anything |

```powershell
cd reverse-e2e/rust-dll
certo build ../a.cto --link strrev.dll.lib -o a.exe; ./a.exe    # fedcba, then 6
```

## `dataprovider/` — a result set as JSON

A DLL that serves database rows to a host. Only `Text` crosses the boundary, which
is the most portable interface: the rows are serialized to a JSON array by hand
(`dataprovider.cto`), with SQL `NULL` as JSON `null`.

```powershell
certo run dataprovider/smoketest.cto    # uses a mocked result set, no database needed
# [{"id":"1","name":"alice","email":"alice@example.com"},{"id":"2","name":"bob","email":null},{"id":"3","name":"dave \"the rock\"","email":"dave@example.com"}]
```

`dataproviderdb.cto` is the real version: it calls `dbConnect` / `dbColumns` /
`dbQuery` and feeds the rows to the same serializer. `dbrun.cto` runs it against
a local PostgreSQL (`certo_demo_users`); that one needs a database and was not
re-run when this README was written.

## Troubleshooting

| Symptom | Fix |
|---|---|
| `E0404: unsafe block ... requires [unsafe]` | add `[unsafe]` (and `io` if it prints) to the function's effects |
| `undefined symbol: rustAdd` | the native library must export the exact extern name |
| the program can't start / `DllNotFoundException` | put every DLL it uses next to the executable |

More in [`docs/DLL-INTEROP.md`](../docs/DLL-INTEROP.md).
