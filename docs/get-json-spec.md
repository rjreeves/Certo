Below is your **single, self‑contained Markdown file**, written as a proper spec document.  
No filler, no chatty tone — just a clean, production‑ready Ion builtin specification.

---

# Ion Builtin Specification: `json.get`

## 1. Purpose

`json.get` is a deterministic, typed, single‑step JSON accessor builtin for the Ion shell.  
It replaces multi‑stage pipelines such as:

```powershell
Get-Content -Raw $MANIFEST |
    ConvertFrom-Json |
    Select-Object -ExpandProperty files |
    Select-Object -ExpandProperty "$path"
```

with:

```ion
json.get $MANIFEST "files.$path"
```

The builtin provides atomic file loading, strict JSON parsing, structured navigation, and Ion‑native type output.

---

## 2. Command Summary

```
json.get <file> <path>
```

- **file** — path to a JSON file (UTF‑8).
- **path** — navigation expression selecting a value inside the JSON structure.

Returns a single Ion value (string, number, bool, null, list, dict).

---

## 3. Path Grammar

```
PATH     := SEGMENT (('.' | '/') SEGMENT | INDEX)*
SEGMENT  := IDENT
INDEX    := '[' INT ']'
IDENT    := [A-Za-z_][A-Za-z0-9_-]*
INT      := [0-9]+
```

### Examples

- `files.$path`
- `files.images.hero`
- `files[0].name`
- `root.section/items[3].id`

Ion variable interpolation occurs before path parsing.

---

## 4. Semantics

### 4.1 File Loading

- Read entire file atomically.
- Reject non‑UTF‑8 input.
- Reject missing or unreadable files.

### 4.2 JSON Parsing

- Parse using a strict JSON parser (e.g., `serde_json`).
- Reject invalid JSON.
- Preserve ordering of object keys.

### 4.3 Navigation Rules

Given `current = root`:

1. For each **SEGMENT**:
   - Require `current` to be an object.
   - Lookup key; fail if missing.
   - Set `current = value`.

2. For each **INDEX**:
   - Require `current` to be an array.
   - Bounds‑check index.
   - Set `current = element`.

### 4.4 Output Type Mapping

| JSON Type     | Ion Type |
|---------------|----------|
| string        | `str`    |
| number (int)  | `int`    |
| number (float)| `float`  |
| boolean       | `bool`   |
| null          | `null`   |
| object        | `dict`   |
| array         | `list`   |

Output is a **typed value**, not text.

---

## 5. Error Model

### Status Codes

| Code | Meaning                    |
|------|----------------------------|
| 0    | Success                    |
| 1    | File error                 |
| 2    | JSON parse error           |
| 3    | Path navigation error      |

### Error Messages

- `json.get: file not found: <file>`
- `json.get: invalid utf-8 in file: <file>`
- `json.get: invalid json in file: <file>`
- `json.get: path not found: <path>`
- `json.get: expected object at segment '<seg>', found <type>`
- `json.get: index <i> out of bounds (len=<n>) at '<seg[i]>'`

Errors are deterministic and never silent.

---

## 6. Examples

### 6.1 Basic Lookup

```ion
let value = json.get $MANIFEST "files.$path"
```

### 6.2 Nested Object

```ion
let hero = json.get "manifest.json" "files.images.hero"
```

### 6.3 Array Element

```ion
let first = json.get "manifest.json" "files[0]"
```

### 6.4 Deep Path

```ion
json.get "config.json" "services.api.endpoints[2].url"
```

---

## 7. Pipeline Integration

`json.get` produces a structured Ion value:

```ion
json.get config.json "database.host" | some::consumer
```

If the consumer expects text, Ion’s normal value‑to‑string conversion applies.

---

## 8. Implementation Sketch (Rust)

### Registration

```rust
builtin_map.add("json.get", json_get, "Get a value from a JSON file by path");
```

### Core Function

```rust
fn json_get(args: &[Str], shell: &mut Shell<'_>) -> Status {
    // 1. Validate args
    // 2. Resolve file path
    // 3. Read file atomically
    // 4. Parse JSON via serde_json
    // 5. Parse path into segments
    // 6. Navigate serde_json::Value
    // 7. Convert final Value -> Ion Value
    // 8. Push into pipeline
    // 9. Return Status::SUCCESS or error
}
```

### Navigation Engine

- Convert path string → token list (`Segment`, `Index`).
- Walk `serde_json::Value` accordingly.
- Convert final value to Ion’s internal `Value` enum.

---

## 9. Optional Extensions

### 9.1 `json.exists`

```
json.exists <file> <path>
```

Returns `true`/`false` without throwing navigation errors.

### 9.2 `json.stream`

```
json.stream <file> <path>
```

Streams array/object elements into the pipeline.

---

## 10. Final Replacement Mapping

**PowerShell:**

```powershell
Get-Content -Raw $MANIFEST |
    ConvertFrom-Json |
    Select-Object -ExpandProperty files |
    Select-Object -ExpandProperty "$path"
```

**Ion:**

```ion
json.get $MANIFEST "files.$path"
```

One builtin. Deterministic. Typed. Restart‑safe.

---

If you want, I can also produce a **companion spec** for `json.exists`, `json.stream`, or a full **json:: module** for Ion.