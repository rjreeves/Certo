Below is a **single, production‑ready Markdown specification** for a **full `json::` module** in Ion — not just a builtin, but a complete structured‑data subsystem with functions, types, error codes, and pipeline semantics.

This is written as if it were part of the official Ion language reference.

---

# Ion Module Specification: `json::`

## 1. Overview

The `json::` module provides deterministic, typed, restart‑safe JSON operations for the Ion shell and Ion scripting language.  
It is designed to replace multi‑stage pipelines, external tools, and ad‑hoc parsing with a coherent, predictable API.

The module exposes:

- **File‑based operations** (`json::get`, `json::exists`, `json::stream`)
- **String‑based operations** (`json::parse`, `json::query`)
- **Serialization** (`json::stringify`)
- **Validation** (`json::validate`)
- **Navigation utilities** (`json::path.split`, `json::path.join`)
- **Error types** (`json::Error`)

All functions return **Ion native types** (`str`, `int`, `float`, `bool`, `null`, `list`, `dict`) and never emit raw text unless explicitly requested.

---

## 2. Module Goals

1. **Deterministic behavior**  
   No silent coercions, no lossy conversions, no ambiguous errors.

2. **Typed output**  
   JSON values map directly to Ion values.

3. **Atomic file operations**  
   Prevent partial reads and race conditions.

4. **Predictable navigation**  
   Strict path grammar, strict type expectations.

5. **Pipeline‑safe**  
   Functions can be used as producers or transformers.

6. **Zero external dependencies**  
   No spawning external JSON tools.

---

## 3. JSON → Ion Type Mapping

| JSON Type     | Ion Type |
|---------------|----------|
| string        | `str`    |
| number (int)  | `int`    |
| number (float)| `float`  |
| boolean       | `bool`   |
| null          | `null`   |
| object        | `dict`   |
| array         | `list`   |

Objects preserve key order.  
Numbers preserve integer/float distinction when possible.

---

## 4. Path Grammar

Used by all navigation functions (`json::get`, `json::exists`, `json::query`, `json::stream`).

```
PATH     := SEGMENT (('.' | '/') SEGMENT | INDEX)*
SEGMENT  := IDENT
INDEX    := '[' INT ']'
IDENT    := [A-Za-z_][A-Za-z0-9_-]*
INT      := [0-9]+
```

Examples:

- `files.$path`
- `root.section/items[3].id`
- `metadata.authors[0].email`

---

# 5. Module Functions

## 5.1 `json::get`

### Signature

```
json::get(file: str, path: str) -> value
```

### Description

Loads a JSON file, parses it, navigates the path, and returns the selected value.

### Errors

- `FileNotFound`
- `InvalidUtf8`
- `InvalidJson`
- `PathNotFound`
- `TypeMismatch`
- `IndexOutOfBounds`

### Example

```ion
let hero = json::get "manifest.json" "files.images.hero"
```

---

## 5.2 `json::exists`

### Signature

```
json::exists(file: str, path: str) -> bool
```

### Description

Returns `true` if the path exists, `false` otherwise.  
Never throws navigation errors.

### Example

```ion
if json::exists "manifest.json" "files.$path" {
    echo "ok"
}
```

---

## 5.3 `json::stream`

### Signature

```
json::stream(file: str, path: str) -> list|dict (streamed)
```

### Description

Streams array or object elements into the pipeline one item at a time.  
Useful for large JSON files.

### Example

```ion
json::stream "manifest.json" "files" | each { echo $it }
```

---

## 5.4 `json::parse`

### Signature

```
json::parse(text: str) -> value
```

### Description

Parses a JSON string into an Ion value.

### Example

```ion
let obj = json::parse '{"name":"Ada","age":36}'
```

---

## 5.5 `json::query`

### Signature

```
json::query(value: any, path: str) -> value
```

### Description

Navigates a JSON/Ion value using the same path grammar as `json::get`.

### Example

```ion
let obj = json::parse $raw
let name = json::query $obj "user.profile.name"
```

---

## 5.6 `json::stringify`

### Signature

```
json::stringify(value: any) -> str
```

### Description

Serializes an Ion value back into JSON text.

### Example

```ion
let text = json::stringify $obj
```

---

## 5.7 `json::validate`

### Signature

```
json::validate(text: str) -> bool
```

### Description

Returns `true` if the string is valid JSON.

### Example

```ion
if json::validate $raw {
    echo "valid"
}
```

---

## 5.8 `json::path.split`

### Signature

```
json::path.split(path: str) -> list
```

### Description

Splits a path into navigation tokens.

### Example

```ion
json::path.split "files.images.hero"
```

Returns:

```
["files", "images", "hero"]
```

---

## 5.9 `json::path.join`

### Signature

```
json::path.join(parts: list) -> str
```

### Description

Joins path segments into a valid JSON path.

### Example

```ion
json::path.join ["files", "images", "hero"]
```

Returns:

```
files.images.hero
```

---

# 6. Error Types

All functions may return a `json::Error` value when failing.

### Error Enum

```
enum json::Error {
    FileNotFound(file: str)
    InvalidUtf8(file: str)
    InvalidJson(file: str)
    PathNotFound(path: str)
    TypeMismatch(expected: str, found: str)
    IndexOutOfBounds(index: int, length: int)
}
```

Errors are deterministic and never ambiguous.

---

# 7. Pipeline Semantics

### Producers

`json::get`, `json::parse`, `json::stream`  
These produce Ion values into the pipeline.

### Transformers

`json::query`, `json::stringify`, `json::path.split`, `json::path.join`

### Consumers

Any Ion command that accepts structured values.

---

# 8. Implementation Sketch (Rust)

### Module Registration

```rust
mod json {
    pub fn get(...) -> Result<Value, Error> { ... }
    pub fn exists(...) -> bool { ... }
    pub fn stream(...) -> Stream<Value> { ... }
    pub fn parse(...) -> Result<Value, Error> { ... }
    pub fn query(...) -> Result<Value, Error> { ... }
    pub fn stringify(...) -> String { ... }
    pub fn validate(...) -> bool { ... }
    pub mod path {
        pub fn split(...) -> Vec<PathToken> { ... }
        pub fn join(...) -> String { ... }
    }
}
```

### Core Engine

- Use `serde_json::Value` internally.
- Convert to Ion `Value` enum.
- Path parser → token list → navigation engine.
- Streaming implemented via iterators.

---

# 9. Design Guarantees

- **No silent coercion**  
- **No lossy number conversion**  
- **Strict path navigation**  
- **Atomic file reads**  
- **Typed pipeline values**  
- **Deterministic errors**  
- **Zero external processes**  

---

# 10. Example: Full Manifest Workflow

```ion
let manifest = json::parse (read $MANIFEST)

let files = json::query $manifest "files"

for file in $files {
    echo $file.name
}
```

Or directly:

```ion
json::stream $MANIFEST "files" | each { echo $it.name }
```

---

If you want, I can also produce:

- a **full Rust implementation skeleton**,  
- a **test suite spec**,  
- or a **formal grammar document** for Ion’s JSON path language.