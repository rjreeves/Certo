# 18. Keeping Your Schema in Sync with the Database

> Status: documents existing, working behavior in `crates/cli/src/main.rs`
> (`certo db pull`, `certo db diff`) and `crates/dbschema`. Nothing in
> this section is a proposal — it's a write-up of a pattern the compiler
> already supports but doesn't currently document anywhere.

## 18.1 The Short Version

Certo's compiler never talks to a live database. Type-checking,
`certo build`, and `certo db pull`'s migration checks are all hermetic —
they only ever read `type` declarations and migration blocks from your
`.cto` source files. Comparing your code against what's *actually* in a
running PostgreSQL database is a separate, explicit step you run when you
want it, using `certo db pull` and `certo db diff`. Nothing forces this
on you, and nothing happens automatically.

This is deliberate, and it's also why builds are reproducible: the same
commit compiles the same way on your laptop, in CI, and on a teammate's
machine, regardless of what anyone's local Postgres instance currently
contains.

## 18.2 The Three Pieces

### 18.2.1 `type` declarations — your committed source of truth

Every `.cto` file with record-typed `type` declarations is, implicitly,
a schema definition:

```certo
type Order = {
    id:        UUID,
    customerId: UUID,
    total:     Decimal,
    placedAt:  Timestamp,
    notes:     Text?
}
```

`Schema::build()` (`crates/dbschema/src/schema.rs`) walks every `type`
declaration with a record body across your compiled module and treats
each one as a table. This is what every other schema-aware check in the
compiler — migration validation, column references — is checked against.
It runs as part of every normal build (`crates/cli/src/main.rs:596`),
with zero network access.

### 18.2.2 `certo db pull` — snapshot what's actually in the database

```sh
certo db pull
# Introspects DATABASE_URL via `psql` and information_schema,
# writes db/schema.cto
```

This connects to your live database (via `psql` — make sure it's on
`PATH`) and writes out a `.cto` file containing `type` declarations that
match exactly what the database currently has: table names, column
names, PostgreSQL types mapped to their closest Certo equivalents (see
§18.4), and nullability. It's ordinary Certo source — nothing special
about the file format, just `type` declarations like any other.

Options:

| Flag | Meaning |
|---|---|
| `-o <file>` | Output path (default: `db/schema.cto`) |
| `--schema <name>` | Which PostgreSQL schema to introspect (default: `public`) |

`DATABASE_URL` is read from the environment or a local `.env` file.

### 18.2.3 `certo db diff` — check for drift, on demand

```sh
certo db diff src/main.cto
```

Compares the `type` declarations reachable from `src/main.cto` against
the *live* database (not the pulled snapshot — this re-queries
`information_schema` directly each time) and reports:

- **MISSING TABLE** — declared as a `type` in your code, absent in the DB
- **MISSING COLUMN** — declared in code, absent in the live table
- **EXTRA COLUMN** — present in the live table, not declared in code
- **TYPE MISMATCH** — declared type doesn't match the live column type
- **NULLABLE DRIFT** — optionality (`?`) doesn't match the live column's
  `NOT NULL` constraint
- **EXTRA TABLE** — exists in the database, isn't declared as a `type`
  at all (informational only — not every table needs a corresponding
  Certo type)

Exit code is `0` when everything matches, `1` when any drift is found —
this is already suitable as a CI gate with no further changes needed:

```sh
certo db diff src/main.cto || exit 1
```

Pass `--json` for machine-readable output (useful for posting drift reports
as PR comments or feeding into other tooling):

```sh
certo db diff src/main.cto --json
```

Output schema:

```json
{
  "in_sync": false,
  "tables_ok": 3,
  "issues": [
    { "kind": "MISSING_COLUMN", "table": "Order", "column": "notes", "detail": "Text?" },
    { "kind": "TYPE_MISMATCH",  "table": "Invoice", "column": "amount", "detail": "code=Int db=Decimal" }
  ]
}
```

Exit code is still `1` when `in_sync` is `false`, regardless of output format.

## 18.3 Putting It Together: the Recommended Workflow

This is the pattern the existing tooling already supports — described
here for the first time, since it isn't written down anywhere else in
the doc set.

**Local development.** Write `type` declarations as your schema source
of truth. Run migrations (`certo db migrate`) to bring your local
database in line with them. You never need `db pull` or `db diff` for
this loop — `certo build` alone keeps your code and your migrations
consistent with each other, hermetically.

**Before committing a schema change.** If you're ever unsure whether
your local database matches what your `type` declarations expect (e.g.
after pulling a teammate's branch, or after manually poking at the DB),
run:

```sh
certo db diff src/main.cto
```

**Onboarding onto an existing database** (one you didn't create the
migrations for — a legacy DB, or a DB owned by another team): use
`certo db pull` to generate a `.cto` snapshot of what's actually there,
then bring the relevant `type` declarations from that snapshot into your
real source files as a starting point, rather than typing them out by
hand.

**CI.** Add `certo db diff src/main.cto` as a step against a staging or
shared database, gated on the exit code. This catches the case the
hermetic build *can't* catch by design: someone ran a manual `ALTER
TABLE`, an out-of-band tool changed the schema, or a migration was
applied inconsistently across environments. Keep this as a separate CI
step from your normal `certo build` — don't fold it into the build
itself, or you reintroduce the non-reproducibility hermetic builds are
meant to avoid.

## 18.4 Important: `db pull` Output Is Not Automatically Used

This is the one piece of behavior worth being explicit about, because
it's easy to assume otherwise: running `certo db pull` writes
`db/schema.cto`, but **nothing reads that file automatically.** Certo's
import resolution (`crates/cli/src/main.rs`, the "Resolve local imports"
step in `cmd_build`) only pulls in files that are explicitly imported by
path from your entry file:

```certo
import db.schema   // pulls in db/schema.cto, IF this import exists
```

Without that `import`, a pulled snapshot is just a file sitting on disk —
useful as a reference or a starting point to copy from, but inert as far
as the compiler is concerned. This is consistent with how every other
import in Certo works (no implicit, directory-convention-based file
discovery), but it means **`db pull` followed by `db diff` is the
intended pairing for drift detection** — `db pull`'s output isn't meant
to silently become part of your build; if you want pulled declarations
to be load-bearing, import them explicitly, the same as any other module.

## 18.5 PostgreSQL → Certo Type Mapping

For reference, this is what `certo db pull`/`certo db diff` use to map
live column types onto Certo types (`crates/cli/src/main.rs`,
`pg_type_to_certo`):

| PostgreSQL type(s) | Certo type |
|---|---|
| `integer`, `int4`, `bigint`, `int8`, `smallint`, `int2`, `serial`, `bigserial`, `smallserial` | `Int` |
| `text`, `character varying`, `varchar`, `char`, `bpchar`, `name`, `citext` | `Text` |
| `boolean`, `bool` | `Bool` |
| `real`, `float4`, `double precision`, `float8` | `Float` |
| `numeric`, `decimal`, `money` | `Decimal` |
| `uuid` | `UUID` |
| `date`, `timestamp`, `timestamp without time zone`, `timestamp with time zone`, `timestamptz` | `DateTime` |
| `json`, `jsonb` | `Text` |
| `bytea` | `Text` |
| anything else | PascalCase of the PostgreSQL type name (best-effort fallback) |

Note `json`/`jsonb`/`bytea` all map to `Text` — there's no dedicated
Certo type for structured or binary blob columns yet, so `db diff`
against a table with a `jsonb` column will compare it as `Text` and
won't catch a mismatch if your `type` declaration uses something else
for that column. Worth knowing if your schema leans on JSON columns
heavily.

## 18.6 Quick Reference

```sh
# One-time: introspect an existing database to bootstrap type declarations
certo db pull -o db/schema.cto

# Ongoing: check for drift before deploying or in CI
certo db diff src/main.cto

# Apply pending migrations (unrelated to pull/diff — this is the
# normal forward-migration path, already covered elsewhere in the docs)
certo db migrate
```
