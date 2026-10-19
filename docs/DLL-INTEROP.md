# Native interop: shared libraries and `extern "C"`

Certo talks to native code in two directions:

| Direction | You write | Certo feature |
|---|---|---|
| **Out** — other languages call Certo | `pub fn` in a `.cto` file, built as a shared library | `certo build --emit-dll`, `certo-ffi --header` |
| **In** — Certo calls other languages | an `extern "C" { ... }` block and an `unsafe { }` call | `certo build --link <lib>` |

The two combine: a Certo library can itself call into a native library, and be
called from C#, Rust, C or anything else that loads a DLL.

A complete, runnable set of examples lives in [`dll-demo/`](../dll-demo/README.md).
Every command in this guide was run against it.

> Shown with Windows names (`.dll`, `.lib`, PowerShell). On Linux and macOS the
> library is `.so` / `.dylib` and there is no separate import library.

---

## 1. Certo as a library (out)

```certo
module Add

/// Adds two integers. Exported to C as `certo_add`.
pub fn add(a: Int, b: Int): Int = a + b
```

```powershell
certo build add.cto --emit-dll -o add.dll    # -> add.dll + add.lib
certo-ffi --header add.cto -o add.h           # -> add.h (C declarations)
```

Only `pub fn` declarations are exported. A file with no `main` builds as a library
(see [HOW-TO-COMPILE](HOW-TO-COMPILE.md) §7); in a project, `type = "lib"` in
`certo.toml` passes `--emit-dll` for you.

### Symbol names

The exported C symbol is `certo_` plus the function name in snake case:

| Certo | C symbol |
|---|---|
| `add` | `certo_add` |
| `reverseString` | `certo_reverse_string` |

Two exceptions:

- **`@export("name")`** on a `pub fn` exports it under exactly that name instead
  (see [CLI-TOOLCHAIN](CLI-TOOLCHAIN.md)).
- A function whose name collides with a symbol the Certo runtime already defines
  (`identity`, `sqrt`, `print`, …) is emitted as `certo_user_<name>`. The header
  generator knows this, so **always take the name from the generated header**
  rather than computing it by hand.

### Type mapping

| Certo | C | Notes |
|---|---|---|
| `Int` | `int64_t` | always 64-bit; `Int8`/`Int16`/`Int32` map to `int8_t`/`int16_t`/`int32_t` |
| `Float` | `double` | `Float32` is `float` |
| `Bool` | `bool` | |
| `Text` | `const char*` | UTF-8, NUL-terminated |
| a record such as `{ reversed: Text, length: Int }` | a struct, returned **by value** | fields in declaration order: `{ const char* reversed; int64_t length; }` |

Lists, options, results and other generic types are not covered by the demos and
have no documented C layout. Keep the exported surface to the types above; pass
anything richer as `Text` (for example JSON — see `dll-demo/dataprovider/`).

### Memory

There is no ownership convention for `Text` across the boundary yet, and no
exported function to release it. Treat a `Text` you receive as borrowed: copy it
if you need to keep it, and don't `free` it yourself. (The Rust demo leaks the
string it returns, with a comment saying so; a real library would also export a
free function.) Text you pass *in* is read during the call.

### Calling it

**C** — include the generated header and link the import library:

```c
#include "add.h"
int main(void) { printf("%lld\n", (long long)certo_add(2, 3)); }
```
```powershell
clang use_dll.c add.lib -o use_dll.exe
```

**C#** — P/Invoke with the exact symbol name; `Int` is `long`:

```csharp
[DllImport("add.dll", EntryPoint = "certo_add")]
static extern long Add(long a, long b);
```

A returned record maps to a `[StructLayout(LayoutKind.Sequential)]` struct, with
`Text` fields as `IntPtr` read through `Marshal.PtrToStringUTF8`.

**Rust** — an `extern "C"` block and `#[link]`:

```rust
#[link(name = "add")]
extern "C" { fn certo_add(a: i64, b: i64) -> i64; }
```

**Certo** — `import Add` compiles the module's source into yours. It does *not*
link the DLL.

---

## 2. Certo calling native code (in)

Declare the foreign function in an `extern "C"` block and call it inside `unsafe`:

```certo
module CallRust

extern "C" {
    fn rustAdd(a: Int, b: Int): Int
}

fn main(): Unit [io, unsafe] = {
    val result = unsafe { rustAdd(2, 3) }
    println(f"rustAdd(2, 3) = {result}")
}
```

```powershell
certo build call_rust.cto --link rustmath.dll.lib -o call_rust.exe
```

Three rules, each enforced by the compiler:

1. **Extern names are used verbatim.** `fn rustAdd` links against the C symbol
   `rustAdd` — not `certo_rust_add`. Spell the native export exactly the same
   way (in Rust, `#[no_mangle] pub extern "C" fn rustAdd` with
   `#[allow(non_snake_case)]`). Earlier versions of Certo mangled these names, so
   older examples that export `certo_rust_add` will fail to link with
   `undefined symbol: rustAdd`.
2. **The call must be inside `unsafe { }`** (error E0210) — the compiler cannot
   check foreign code.
3. **The enclosing function must declare the `[unsafe]` effect** (error E0404),
   which then propagates to its callers. Add `io` as well if it prints:
   `[io, unsafe]`. A safe wrapper that handles the result can keep the effect
   confined to one function.

`--link` takes a library path and may be repeated. The native library must be
findable at run time (next to the executable on Windows).

### Putting it together

A Certo library can call native code *and* be called from another language. In
`dll-demo/reverse-e2e/`, C# calls a Certo DLL, which calls a Rust DLL:

```
C# (Program.cs) ──► strutil.dll (Certo) ──► strrev.dll (Rust)
```

```powershell
certo build strutil.cto --emit-dll --link rust-dll/strrev.dll.lib -o strutil.dll
```

Ship both DLLs next to the C# executable.

---

## 3. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `error[E0404]: unsafe block in 'f' requires [unsafe]` | Add the effect: `fn f(): T [unsafe] = ...` |
| `error[E0210]` | The `extern` call is outside an `unsafe { }` block |
| `lld-link: error: undefined symbol: rustAdd` | The native library doesn't export that exact name, or `--link` wasn't given. Extern names are verbatim |
| `DllNotFoundException` / the program can't start | The DLL (and any DLL it depends on) isn't next to the executable |
| `EntryPointNotFoundException` | The `EntryPoint` doesn't match the exported symbol — check `add.h` |
| Garbled text from a returned record | The struct layout is wrong: fields must be in declaration order, `Int` is 64-bit (`long`), `Text` is a pointer |

---

## See also

- [`dll-demo/`](../dll-demo/README.md) — the runnable examples for everything above
- [CLI-TOOLCHAIN](CLI-TOOLCHAIN.md) — `--emit-dll`, `--link`, `@export`
- [HOW-TO-COMPILE](HOW-TO-COMPILE.md) — building libraries and projects
- [ERROR-REFERENCE](ERROR-REFERENCE.md) — E0210, E0404
- [GUIDE](GUIDE.md) §11 — a short introduction
