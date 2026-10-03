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
  also in front of `insert` / `update` / `delete`.

All three run against PostgreSQL and SQLite in the live tests, and the generated C# (below)
handles them.

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

## Not covered yet

- No recursive `with`, named window definitions (`window w as (...)`), frame `exclude`
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
