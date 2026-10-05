# certo-runner

Project files, migration history and execution for Certo schemas.

```
certo sdl migrate init                 # certo-db.toml, schema.sdl, IR.json, migrations/
# edit schema.sdl
certo sdl migrate new add_slug --mdl add_slug.mdl
# review migrations/0002_add_slug/up.sql
certo sdl migrate apply --dry-run      # print the SQL, touch nothing
certo sdl migrate apply                # run it (DATABASE_URL or --url)
certo sdl migrate status
certo sdl migrate drift                # does the live database still match the migrations?
certo sdl migrate adopt                # bring an EXISTING database under management
```

## On disk

| Path | Role |
|------|------|
| `certo-db.toml` | dialect, schema path, migrations directory (no credentials) |
| `schema.sdl` | the schema you edit |
| `IR.json` | the schema as of the last generated migration |
| `migrations/NNNN_name/up.json` | **the executable SQL batches**, checksummed |
| `migrations/NNNN_name/up.sql` | readable copy of `up.json`; never executed |
| `migrations/NNNN_name/plan.json`, `ir.json`, `migration.mdl` | provenance |

Commit all of it. `up.json` is frozen when a migration is created: upgrading
the compiler later cannot change what an already-reviewed migration does.
Hand-written steps belong in MDL (`--mdl`), not in `up.sql`.

## Guarantees

- **Atomic:** each transactional batch runs in one transaction and the history
  row is written inside it, so a migration is fully applied and recorded, or
  not at all. The only exception is PostgreSQL's `ADD VALUE` for enums, which
  cannot share a transaction with code that uses the value; it runs first as
  a non-transactional batch and is idempotent (`IF NOT EXISTS`).
  This holds on SQLite for a table rebuild too: its script ends with `PRAGMA foreign_keys = ON`, which cannot run in a
  transaction, so the history row goes into the last transactional batch, not after the script (a failed migration also
  puts that pragma back).
- **Tamper-evident:** the SHA-256 of `up.json` is stored in the history table.
  Before anything runs, the history must be an exact prefix of the files on
  disk: an edited, renamed, missing or out-of-order migration is refused.
- **The journal (opt-in):** with a `JournalContext` (`ApplyOptions::journal`, `AdoptOptions::journal`; `journal` in the C ABI
  options), each applied migration and each adopted baseline adds a row to the append-only table `_certo_log` (id, at, action,
  subject, actor, environment, tool, detail) INSIDE the transaction that makes the change, so the journal holds exactly the
  changes that committed: a migration that rolled back leaves no row, and one that committed cannot be missing one. The table is
  created on first use, is certo's own (ignored by introspection, so it is not drift and is not imported), and is off unless a
  host asks. Drift repair scripts are only printed, never run by certo, so they are not journaled.
- **`new` survives being killed:** a migration is built in a hidden folder (`migrations/.0003_x.tmp`) with each file flushed
  to disk, then moved into place in one step; `IR.json` is likewise written beside itself and replaced in one step. So a
  process killed part-way never leaves a folder with some of its files (which would make every command fail reading it):
  a leftover hidden folder is not part of the history and is removed by the next `new`. A kill after the folder was
  published but before `IR.json` followed leaves `IR.json` exactly one step behind; the next `new` recognises that one
  state, brings `IR.json` forward (so it reports "no changes" rather than creating a duplicate), and still refuses any
  other edit of `IR.json`.
- **Adopt survives being killed:** while `adopt` works, the project holds a `.certo-adopt` marker (it contains the
  `schema.sdl` that was there), written before the first file and removed after the baseline is recorded. If the process
  is killed in between, the next `adopt` finds the marker and settles it first: if the database already holds exactly this
  adopt's baseline, only the marker is cleared; if the database has no history, the partial files are discarded, `schema.sdl`
  is put back and the adoption is redone from scratch (it is computed from the database, so the result is the same);
  if the history is something else, nothing is touched. The report's `recovered` says which. A dry run refuses while a marker
  is there rather than guess.
- **Serialised:** a session advisory lock (PostgreSQL) or a file lock on `<database>.certo-lock` (SQLite) stops two
  runners interleaving; the second fails at once instead of waiting.
- **Read-only where it should be:** `status` and `apply --dry-run` never
  create the history table.
- **Destructive changes need consent:** `new` refuses a plan that can lose
  data unless `--allow-destructive` is given.

- **Views:** `views.ql` (see the QL README) is part of the project. With migrations to run, `apply` drops the views certo
  recorded in `_certo_views`, runs them, and creates the views again in one transaction; it also recreates views whose definition
  changed or that were dropped by hand. `status` reports them (`views`: defined, recorded, in_sync) and a mistake in `views.ql`
  as `views.error`; `apply` refuses to start with one. Drift reports a missing view, a changed one, and one no longer in the
  file; views certo manages are not "left out" notes.

- **SDL views:** `view name on table (columns) where ...` in `schema.sdl` is part of the schema and so of the migrations:
  a changed or removed view, or one over a table whose columns change (or are renamed), is dropped before the migration's
  other changes and created again after them. Drift compares presence only (a database does not hand a view back as it was
  written): a declared view that is missing is drift; one that is there is not "left out". `views.ql` views may read them.

## MySQL

A project with `dialect = "mysql"` runs against MySQL 8 (`mysql://user:password@host:port/database`; the URL must name the database,
and `--yes`-style safety is the same as for the others). Migrations are written by `migrate new` as for the other dialects (see
`certo-sql`'s MySQL notes for what is refused and why).

MySQL commits every DDL statement as it runs, so a migration cannot be atomic. The runner applies it **one statement at a time and
counts**: `_certo_progress` holds how many of a migration's statements have run. If one fails, the ones before it are in the
database, `status` lists the migration under `partial` (`seq`, `done`, `total`), the error says which step failed, and `apply` again
carries on from that step once the cause is fixed. The history row (and journal row) is written, in one transaction with removing the
progress row, only after the last statement succeeded. A migration whose files changed while half applied is refused. If the runner
is killed between a statement and its progress row, that statement runs again on resume and (for DDL) reports "already exists" for
you to settle by hand. Views (`views.ql`) are dropped and created one statement at a time with the record kept true at every step.

The connection is UTC with `utf8mb4`, and the runner takes a named lock (per database) so two runs cannot interleave. Reading a schema
back (drift, `schema import`, `adopt`) uses `information_schema` and the column comments the DDL leaves (`certo:enum Name`,
`certo:uuid`, `certo:timestamptz`); a database made by hand is read by its types, and what SDL cannot express (unsigned or
`tinyint` columns other than `tinyint(1)`, multi-column keys, fulltext indexes, triggers, unnamed enums) is left out and said so.
`RESTRICT` and `NO ACTION` are one thing to InnoDB, as are `serial` and `generated by default` (`AUTO_INCREMENT`).

## Drift detection

`certo sdl migrate drift` reads the live schema back from the PostgreSQL
catalog (enums, composite types, tables, columns, primary/unique/foreign keys
with their actions, indexes, CHECK constraints, defaults) and compares it with
the `ir.json` of the **last applied** migration (not `IR.json`, which may
include migrations you have not applied). Findings are grouped as *missing
from*, *different in* and *unexpected in* the database, and the exit status is
1 when there is drift, so it works in CI.

- `--sql` prints the script that would bring the database back to the
  migrations. It is never executed for you: review it.
- `--json` gives a machine-readable report.
- `apply --check-drift` refuses to run new migrations on a drifted database.
- Live objects SDL cannot express (`varchar(n)`, multi-column unique or foreign
  keys, unique/partial/expression indexes, identity columns) are listed as
  `note:` lines and are **not** counted as drift.
- Defaults and CHECK bodies are compared as normalised text (quotes,
  parentheses, whitespace, `::casts` and case ignored). That catches changed
  literals, operators, columns and functions, but not a change that only alters
  operator grouping. Treat "no drift" as strong, not absolute, evidence for
  expressions.
- Relationships have no database form and are never reported.
- Scope is the connection's current schema.

## Adopting an existing database

`certo sdl migrate adopt` (run after `init`, in a fresh project) turns a
database you already have into a managed one:

1. reads its schema back (the same introspection drift uses);
2. translates live defaults and CHECK bodies into SDL expressions;
3. drops what SDL cannot express, together with everything that depends on it;
4. writes `schema.sdl`, recompiles it and requires the **exact same IR**;
5. freezes a `0001_baseline` migration and **records it as applied without
   running it**. The baseline holds the real `CREATE` statements, so an empty
   database can be rebuilt from the history; on the adopted database it is
   only history.

Use `--dry-run` first: it prints the `schema.sdl` it would write and lists every
omission with its reason, changing nothing. Adoption refuses a project that
already has migrations, a `schema.sdl` with declarations (unless `--force`),
and a database that already has a migration history.

**What is left out** (each reported as `not adopted`, with why): the types
SDL has no name for (`timestamptz(3)`, `interval`, `inet`, arrays, ranges, ...);
defaults and CHECKs SDL cannot express (exponent-form numbers, most functions
and operators such as `~` and `||`, casts of non-literals, a `nextval` of a
sequence that was itself left out); multi-column unique and foreign keys;
unique, partial and expression indexes; generated (computed) columns; and any
table, enum, sequence or column whose name SDL cannot spell, along with the
keys, indexes and checks that used it.

`serial` / `bigserial` / `smallserial` columns, identity columns and standalone
sequences are adopted: a serial key becomes `id: serial primary key`, an
identity column `generated always` or `generated by default`, and a shared
sequence a `sequence` declaration that its `nextval(...)` defaults refer to.
Sequences that belong to a column (the ones serial and identity create) are not
declared separately.

Since SDL grew negative and decimal literals, `not`, `is [not] null`, `in`,
`varchar(n)` / `char(n)` / `decimal(p,s)`, `smallint`, `real`, `timestamp_naive`
and a few more functions, all of those are adopted rather than left out.

Every translation must pass two independent checks or it is left out: it must
compile as real SDL, and its rendering back to SQL must normalise to the same
text as the server's. Nothing is guessed.

What you left out still exists in the database, so `drift` keeps listing it
until you add it to `schema.sdl` or remove it from the database. Adoption
prints that list too, so you know it is expected.

## Not covered yet

- No **down migrations**; roll forward with a new migration.
- SDL itself still limits adoption: no `interval`, network types or arrays,
  and a small function set.
- A serial column's sequence options (start, increment, cache) are not
  recorded: a serial column is a plain counter. Identity options likewise.
- Sequence `last_value` (where the counter is) is state, not schema, and is
  never compared. Adding serial or identity to a column with rows catches the
  new counter up to the largest existing value.
- PostgreSQL and SQLite (`dialect = "sqlite"`; `--url app.db`, `sqlite:<path>` or `:memory:`). SQLite has
  no advisory lock, so the runner takes an operating-system lock on a sidecar file, `<database>.certo-lock`, while it is
  connected (released when the process ends, however it ends; the empty file stays, so you may want to ignore
  `*.certo-lock` in version control; an in-memory database, or one in a folder that cannot be written, is not locked); its
  reader can only see types the SQLite adapter's spellings (`UUID TEXT`, `ENUM_Role TEXT`, ...)
  identify, so a database not created by Certo reports other types as not represented. `sslmode=require` selects TLS but has not been exercised
  against a TLS-enabled server.
- The history table and lock key are fixed (`_certo_migrations`).

## Testing against a live server

The introspection tests are skipped unless `CERTO_TEST_PG_URL` is set. They
**drop and recreate the `public` schema**, so use a throwaway database:

```
CERTO_TEST_PG_URL=postgres://user@localhost/scratch cargo test -p certo-runner --test live_pg -- --test-threads=1
```
