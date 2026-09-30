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

## Not covered yet

- No subqueries, `union`, window functions or CTEs; no `insert ... select`
  or multi-row inserts.
- Dialects: PostgreSQL and SQLite (`--dialect sqlite`; see `crates/sql/README.md` for the
  differences). A small function set (see `docs/ebnf.md`).
- A `case` cannot mix a string literal with an enum column in its branches.

## Testing against a live server

```
CERTO_TEST_PG_URL=postgres://user@localhost/scratch cargo test -p certo-ql --test live_ql
```
**Drops and recreates the `public` schema**: use a throwaway database.
