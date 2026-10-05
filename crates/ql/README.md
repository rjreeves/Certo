# certo-ql

QL: typed, parameterised queries and mutations (`insert` / `update` / `delete`),
checked against a Certo schema and compiled to SQL.

```
query recent_orders(min_total: decimal(10,2), since: timestamp null) {
    from orders o
    left join customers c on o.customer_id == c.id
    where o.total >= :min_total and o.created > :since
    select o.id, c.name as customer, o.total
    order by o.total desc
    limit 50
}
```

```
certo ql check   queries.ql --schema schema.sdl     # types of every parameter and result column
certo ql compile queries.ql --schema schema.sdl     # the SQL, with $1/$2 placeholders
certo ql compile queries.ql --schema IR.json --json # machine-readable
```

`check` prints each query's contract:

```
query recent_orders(min_total: decimal(10,2), since: timestamp?)
  -> id int, customer text?, total decimal(10,2)
```

(`?` marks nullable.) `customer` is nullable because it comes from the far side
of a `left join`.

## What a compiled query is

For each query the compiler produces

- **SQL** with numbered placeholders, each cast to its declared type
  (`$1::numeric(10,2)`), so PostgreSQL never has to guess;
- **`param_order`**: which declared parameter is `$1`, `$2`, ... (numbered by
  first use, and reused when a parameter appears twice);
- **typed result columns**: name, type and nullability, known before the query
  runs;
- the fully resolved **`ir`**.

The host never assembles SQL from strings: it binds parameters in
`param_order` and knows the shape of the rows it will get back.

From C# (`CertoNative.CompileQl(schemaIrJson, qlSource)`) the result is JSON;
see `crates/capi/include/certo.h`. Errors are positioned diagnostics with codes
`QL2xx` (`QL206` unknown column, `QL211` type mismatch, `QL214` column not
grouped, ...); every independent error in a file is reported in one pass.

## Guarantees

- Names, types, aggregates and grouping are checked against the schema.
- The declared result types and parameter types are verified against a real
  PostgreSQL in `tests/live_ql.rs`: each query is prepared and executed, the
  column and parameter types PostgreSQL reports must equal the declared ones,
  and no column declared NOT NULL may ever contain NULL.
- Expression size, depth and nesting are bounded, so hostile input is refused
  rather than crashing the compiler.

## Mutations

```
insert add_customer(name: text, email: varchar(100) null) {
    into customers set name = :name, email = :email
    on conflict (email) do update set name = excluded.name
    returning id, name
}
update rename(id: int, n: text) { customers c set name = :n where c.id == :id returning c.id }
delete purge(before: timestamp) { from orders o where o.created < :before }
delete wipe_items() { from items i all rows }
```

Same parameters, expressions and checker as queries, plus write-specific rules:
an `update` / `delete` needs `where` or an explicit `all rows`; a value that may
be NULL is refused for a NOT NULL column (QL232); an insert must supply every
required column (QL233); `generated always` columns cannot be set (QL234); no
column is set twice (QL235); `on conflict` needs a real unique key (QL236);
values are stored without narrowing (an `int` parameter does not fit a
`smallint` column; literals and `qty + 1` do) and enum literals are validated.
`returning` gives the same typed contract as `select`; without it the host
gets a row count. Placeholders are numbered by first use in the SQL text.

## Subqueries and bulk inserts

```
query active() {
    from customers c
    where exists (from orders o where o.customer_id == c.id select 1)
      and c.id not in (from blocked b select b.customer_id)
    select c.name,
           (from orders o where o.customer_id == c.id select count(*)) as orders,
           (from orders o where o.customer_id == c.id select o.status order by o.id desc limit 1) as latest
}
insert add_two(a: text, b: text) {
    into customers (name, email) values (:a, "a@x.com"), (:b, "b@x.com") returning id
}
insert snapshot() { into archive (id, total) from orders o where o.paid select o.id, o.total }
```

A subquery is a query's own clauses in parentheses. It sees the tables around it (correlation),
inner names winning. `in` and value subqueries need one column; a value subquery must be
provably one row (aggregates without `group by`) or say `limit 1`, and is then typed like the
aggregate, or nullable. Tabular inserts check every row like a `set` value and count the values
against the column list. Both are run against PostgreSQL and SQLite in the live tests.

## Window functions, set operations and `with`

```
query ranked() {
    with spend as (from orders o group by o.customer_id select o.customer_id, sum(o.total) as total)
    from customers c join spend s on s.customer_id == c.id
    select c.name, s.total,
           rank() over (order by s.total desc) as place,
           lag(s.total) over (order by s.total desc) as next_up
    order by place
}
query people() {
    from customers c select c.id, c.name
    union all
    from staff s select s.id, s.name
    order by id limit 100
}
```

- **Window functions:** `count`/`sum`/`avg`/`min`/`max`, `row_number`, `rank`, `dense_rank`,
  `ntile`, `percent_rank`, `cume_dist`, `lag`, `lead`, `first_value`, `last_value`, with
  `over (partition by ... order by ...)`. Typed like the aggregates; ranking functions never
  return NULL; `lag` / `lead` do unless given a default. Only in `select` and `order by`.
  Frames: `sum(o.total) over (order by o.id rows between unbounded preceding and current row)`
  (running total), `rows between 2 preceding and 2 following` (moving window), `range` by value,
  `groups` of equal keys; offsets are literals or parameters.
- **Set operations:** `union`, `union all`, `intersect`, `except` between full `from ... select`
  branches. Columns merge their types and nullability; `order by` / `limit` at the end apply to
  the whole result and name its columns.
- **`with`:** named queries used like tables by the body, later `with` queries, and subqueries;
  also in front of `insert` / `update` / `delete`. `with recursive` walks hierarchies: a starting
  select, then `union [all]` steps that read the query itself (types must match the start; no
  aggregates in a step; bound a cyclic graph with a depth limit or use `union`):

  ```
  with recursive tree as (
      from categories c where c.parent_id is null select c.id, c.name, 0 as depth
      union all
      from categories c join tree t on c.parent_id == t.id select c.id, c.name, t.depth + 1 as depth)
  from tree t select t.name, t.depth order by t.depth, t.name
  ```

All three run against PostgreSQL and SQLite in the live tests, and the generated C# (below)
handles them.

## Text and date functions

`substr(text, 2)`, `substr(text, 1, 3)` (literal start >= 1 and length >= 0), `replace(text, "a", "b")`,
`position(text, "x")` (1-based, 0 if absent) and `date_part("year", d)` (`year`, `month`, `day`; `hour`, `minute` for
timestamps) behave the same on PostgreSQL and SQLite; `position` lowers to `strpos`/`instr` and `date_part` to
`EXTRACT`/`strftime`, always giving an `int`. A timestamp with a time zone is read in UTC, as SQLite stores it,
whatever the PostgreSQL session's zone is.

`left(text, 2)`, `right(text, 3)` (literal lengths; `right` needs 1 or more), `starts_with(text, "ab")`,
`add_days(date, 7)` (a date and a whole number of days, negative to go back) and `days_between(from, to)` (whole days,
`to - from`) are the same on both databases. There is no month or year arithmetic: Jan 31 plus one month ends differently.

## Fragments

```
fragment paid_orders() { from orders o where o.paid select o.id, o.customer_id, o.total }
query q() { from paid_orders p join customers c on p.customer_id == c.id select c.name, p.total }
```

A `fragment` is a named query used as a table, anywhere a table can be (including in subqueries and in the
source or filter of a mutation). It is inlined as a `with` query of each statement that uses it: no migration, no database
object. Fragments can use other fragments, are checked once on their own, and cannot be written to (`QL260`).

A fragment can take parameters, filled in where it is used with literals or the statement's own parameters (never columns):

```
fragment since(cutoff: timestamp) { from orders o where o.created >= :cutoff select o.id, o.customer_id, o.total }
query recent(at: timestamp) { from since(:at) r join customers c on r.customer_id == c.id select c.name, r.total }
query fixed() { from since("2026-01-01") r select count(*) as n }
```

Each distinct call becomes its own `with` query (equal calls share one). The alias defaults to the fragment's name. A wrong number
of arguments, a column or expression as an argument, a parameter of another type or one that may be null where the fragment's
isn't, are `QL260`.

## Views

```
view adult_customers { from customers c where c.age >= 18 select c.id, c.name }
view big_spenders { from adult_customers a join orders o on o.customer_id == a.id group by a.id select a.id, sum(o.total) as spent }
```

A `view` is a fragment that is kept in the database as a `CREATE VIEW`. Put views in the project's `views.ql` (`"views"` in
`certo.json` changes the path); `migrate apply` manages them. Queries read a view like a table, other views can read it (they are
created in dependency order; a cycle is `QL262`), and a view cannot be written to (`QL263`). Views take no parameters. Fragments
may be used inside views. Because a view stops a table it reads from being altered, `apply` drops the views before running
migrations and creates them again afterwards; it records each view's checksum in `_certo_views`, so an `apply` that changes
nothing leaves them alone. `ql_compile` lists them (`views`: name, columns, `create_sql`, `drop_sql`, and the view as a table).

## Filtered aggregates and `string_agg`

`count(*) filter (where o.paid)` counts only matching rows (any aggregate, also over a window). `string_agg(c.name, ", " order by
c.id)` joins text values (NULLs skipped; `NULL` for an empty group; `group_concat` on SQLite). `filter` needs SQLite 3.30,
and an `order by` inside `string_agg` SQLite 3.44. Misuse is `QL259`.

## JSON keys

`json_text(c.meta, "plan")`, `json_int(c.meta, "limits", "seats")`, `json_bool(c.meta, "trial")`, `json_has(c.meta, "tags", 0)`:
read a `json` value along a literal path of one to four keys (names, or array positions). Each returns only what both databases
agree on: a JSON string as text, a whole number below 10^18 as `bigint`, `true`/`false` as a boolean, and NULL for a missing key,
a value of another kind, JSON `null` or a NULL column (`json_has` is false instead of NULL). Text that is not valid JSON reads as
NULL. Keys are literals, not parameters or columns (`QL209`).

## Named windows

```
from orders o
window w as (partition by o.customer_id order by o.id)
select o.id, row_number() over w as n, sum(o.total) over w as running
```

`window name as (...)` goes after `having` and before `select`; `over name` is exactly the definition written out, in
`select` and `order by`. Unknown or repeated names are `QL258`. (`intersect all` / `except all` are not offered: SQLite
has no such operators.)

## Joining text

`a || b || c` concatenates text (NULL if any part is). Besides text, whole numbers, enums and uuids may be
joined (they print the same in PostgreSQL and SQLite); other types cannot. It binds like `+`. Together with
recursive `with` it builds paths:

```
select t.path || "/" || c.name as path      -- inside the recursive step
```

## Generated host code (C#)

```
certo ql codegen queries.ql --schema schema.sdl --lang csharp --dialect sqlite --namespace App.Db -o Queries.cs
```

The typed contract becomes C#: a `sealed record <Name>Row` per statement that returns rows
(nullable columns are `T?`), and an extension method `<Name>Async(this DbConnection, ...)`
per statement with one C# parameter per QL parameter (`T?` for `null` ones), an optional
`DbTransaction` and a `CancellationToken`. A query or a mutation with `returning` gives
`List<Row>`; any other mutation gives the affected-row count. Enums become C# enums (with
their database text in a generated `<Enum>Text` class), and every statement's SQL is also a
constant. It is plain ADO.NET, so it runs on Npgsql and Microsoft.Data.Sqlite; under
PostgreSQL, enum result columns are selected as `text` so drivers need no enum mapping.
Type mapping: `timestamp` is `DateTimeOffset`, `timestamp naive` `DateTime`, `date`
`DateOnly`, `uuid` `Guid`, `json` the JSON text, `decimal` `decimal`. Under SQLite,
timestamps are stored as the driver writes them, which differs from `now()`'s form: compare
them in the application.

`packaging/dotnet/codegen-check.ps1` generates code from `Certo.Codegen.Check/sample`, compiles
it, and runs it against SQLite (always) and PostgreSQL (when `CERTO_TEST_PG_URL` is set). The same
generator is available to hosts as `certo_ql_codegen` in the C ABI.

## Generated host code (Rust)

```
certo ql codegen queries.ql --schema schema.sdl --lang rust --dialect sqlite -o queries.rs
```

Plain, synchronous driver code, one driver per dialect: `--dialect postgres` (the default) calls the `postgres` crate and takes
any `postgres::GenericClient` (a `Client` or a `Transaction`); `--dialect sqlite` calls `rusqlite` and takes a `&Connection`
(which a `Transaction` derefs to). Each statement is a function named after it in `snake_case`, with one Rust parameter per QL
parameter (`Option<T>` for `null` ones; text and bytes are borrowed as `&str` and `&[u8]`; a parameter the statement never uses
keeps its place and gets a leading `_`). A statement that returns rows gives `Vec<<Name>Row>`, a struct with one field per column;
any other mutation gives the affected-row count (`u64`). The SQL is also a constant (`<NAME>_SQL`). Enums become Rust enums with
`as_db()`/`from_db()` that read and bind as their database text; `json` is a generated `Json(String)`; `decimal` is
`rust_decimal::Decimal`, `uuid` is `uuid::Uuid`, `timestamp` is `chrono::DateTime<Utc>`, `timestamp naive` and `date` are chrono's
naive types. The first lines of the file list the crates and features it needs. Under SQLite, timestamps are written the way
`CURRENT_TIMESTAMP` is (UTC, no offset), decimals are bound as text and read from whichever number or text SQLite holds, and
uuids are text. The generated code is compiled and run against live SQLite and PostgreSQL in `tests/rust_codegen.rs`.

## Not covered yet

- No named window definitions (`window w as (...)`), frame `exclude`
  clauses, or `intersect all` / `except all`; subqueries cannot appear in `limit` / `offset`; an `insert`
  cannot take a variable number of rows from one parameter.
- Dialects: PostgreSQL and SQLite (`--dialect sqlite`; see `crates/sql/README.md` for the
  differences). A small function set (see `docs/database/ebnf.md`).
- A `case` cannot mix a string literal with an enum column in its branches.

## Testing against a live server

```
CERTO_TEST_PG_URL=postgres://user@localhost/scratch cargo test -p certo-ql --test live_ql
```
**Drops and recreates the `public` schema**: use a throwaway database.
