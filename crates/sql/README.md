# certo-sql

Lowers an engine-agnostic `MigrationPlan` to SQL. Dialects: `postgres`, `sqlite`.

```rust
// PostgreSQL lowers from the plan alone
let script = certo_sql::render(&plan, Dialect::Postgres)?;
// SQLite also needs the schemas the plan was made between
let script = certo_sql::render_with(&plan, Dialect::Sqlite, Schemas { old: &old, new: &new })?;
```

## SQLite (3.35+)

- **Types** map to affinities: `uuid`, `json`, `date`, `timestamp` are `TEXT` (ISO
  strings), `bool` is `BOOLEAN` (0/1), `bytes` is `BLOB`, decimals are `NUMERIC`.
  An enum is `TEXT` with an inline `CHECK (col IN (...))`.
- **Rebuilds.** `ALTER TABLE` cannot change a column, add or drop a foreign key or
  CHECK, or add a NOT NULL / UNIQUE column, so those changes rebuild the table
  from its shape in the new schema: create the new table, copy the rows (NULLs in
  a column made NOT NULL take its default), drop, rename, recreate indexes. One
  rebuild absorbs every structural change to that table in the plan. Rebuilds run
  as three batches: `PRAGMA foreign_keys = OFF`, the transaction, `PRAGMA
  foreign_keys = ON` (SQLite ignores the pragma inside a transaction). Simple
  changes (add a nullable or constant-default column, renames, indexes, data
  updates) stay plain `ALTER TABLE`.
- **Generated values** are `INTEGER PRIMARY KEY AUTOINCREMENT`: `serial` and both
  identity forms map to it, and only on a single-column integer primary key.
  `generated always` is not enforced.
- **Refused, with a reason:** sequences and `nextval`, composite types, generated
  columns that are not the integer primary key.
- `now()` is `CURRENT_TIMESTAMP` (UTC, `YYYY-MM-DD HH:MM:SS`); `gen_uuid()` builds a
  version-4 UUID from `randomblob`.
- A type change does not convert stored values (SQLite is dynamically typed); the
  new column's affinity applies as rows are copied.

QL compiles for SQLite too (`?1` placeholders wrapped in `CAST`, `RETURNING`
without table qualifiers, `LIMIT -1` before a bare `OFFSET`). Differences to know:
`LIKE` is case-insensitive for ASCII, decimals are stored as floats/integers, and
`avg` of integers is a float.

`tests/sqlite_live.rs` runs the scripts on a real SQLite, including evolving a
schema with data and comparing the result with a fresh database.

The runner applies SQLite migrations, reads a SQLite database back into a schema
(declared type names carry the distinction: `UUID TEXT`, `DATE TEXT`, `ENUM_<name> TEXT`, ...),
detects drift and adopts existing databases; see `crates/runner/README.md`.
