# Introduction to Database Programming with Certo

> A short primer on talking to PostgreSQL from Certo — connecting, querying,
> mutating, migrating, testing, and exploring live data.

Every piece of syntax in this guide has been checked against the real
compiler (`certo check` / `certo run`) while writing it — nothing here is
aspirational. Where the language spec (`docs/CERTO-SPEC.md` §6.1–6.4)
describes a different, more fluent-looking database API, that section
documents a *design sketch* that was never implemented; ignore it. This
guide describes what actually ships.

---

## Table of Contents

1. [The Certo Database Story](#1-the-certo-database-story)
2. [Connecting to PostgreSQL](#2-connecting-to-postgresql)
3. [Raw Queries and Execution](#3-raw-queries-and-execution)
4. [SQL Injection Protection](#4-sql-injection-protection)
5. [Transactions](#5-transactions)
6. [Mapping Rows to Types](#6-mapping-rows-to-types)
7. [The Typed Query DSL](#7-the-typed-query-dsl)
8. [The Mutation DSL](#8-the-mutation-dsl)
9. [Migrations](#9-migrations)
10. [Testing Database Code](#10-testing-database-code)
11. [Exploring Live Data with the REPL](#11-exploring-live-data-with-the-repl)
12. [A Small Worked Example](#12-a-small-worked-example)
13. [Limitations and Where to Go Next](#13-limitations-and-where-to-go-next)

---

## 1. The Certo Database Story

Certo talks to **PostgreSQL only**, over `libpq`. A Certo program that uses
the database stdlib compiles that support straight into the native binary —
there's no ORM process, no driver to install separately, no connection pool
service. `dbConnect` opens one real libpq connection; you use it directly.

There are three layers, and you can mix them freely in the same program:

| Layer | What it is | When to reach for it |
|---|---|---|
| **Raw queries** (`dbQuery`, `dbExec`, …) | Thin wrappers over `PQexecParams`/`PQexec` | Anything ad hoc, scripts, one-offs |
| **Typed Query/Mutation DSL** (`Query.*`, `Mutation.*`) | A small builder checked at *compile time* against your `type` declarations | Application code where you want the compiler to catch a typo'd column name |
| **Migrations** (`migration { ... }` files) | Versioned schema changes, applied via `certo db migrate` | Setting up and evolving your schema |

All three share one thing: SQL parameters always travel as `List<Text>`,
bound positionally as `$1`, `$2`, `$3`, … — Certo doesn't have a native
`Int`/`Decimal`-typed parameter binding, so you convert values to `Text`
yourself (`intToText`, `floatToText`, and so on) before they go in the list.

---

## 2. Connecting to PostgreSQL

```certo
module DbHello

import Stdlib.Core
import Stdlib.Db

fn main(): Unit [io] = {
    val connstr = arg(1) ?? "host=localhost dbname=postgres user=postgres"
    val conn = dbConnect(connstr)
    if conn == 0 then {
        eprintln(f"could not connect: {dbError(0)}")
    } else {
        println("connected!")
        dbClose(conn)
    }
}
```

`dbConnect(connstr: Text): Int` returns a connection handle — an opaque
`Int` — or `0` on failure. The connection string accepts either libpq's
`key=value` form (`host=localhost dbname=mydb user=myuser password=secret`)
or a `postgres://`/`postgresql://` URI; both are accepted everywhere a
connection string is expected, including on the CLI.

A handful of other connection-level functions round this out:

```certo
dbClose(conn: Int): Unit
dbError(conn: Int): Text            // last error message; works even for conn == 0
dbServerVersion(conn: Int): Int     // e.g. 160000 for PostgreSQL 16
dbVersionString(conn: Int): Text
```

There's no connection pool built in — every `dbConnect` call is one
synchronous, blocking connection. For short-lived scripts and CLI tools
that's usually exactly what you want.

### Where the DSN comes from

Hardcoding a connection string is fine for a five-line example; real
programs read it from the environment:

```certo
val dsn = getEnv("DATABASE_URL") ?? panic("DATABASE_URL is not set")
val conn = dbConnect(dsn)
```

This is also what the Certo *toolchain itself* does — `certo db migrate`,
`certo db pull`, `certo db diff`, and `dbTest` (§10) all look for
`DATABASE_URL`, first as an environment variable, then in a `.env` file in
the current directory:

```
# .env  (gitignored — certo new scaffolds a .env.example alongside it)
DATABASE_URL=host=localhost dbname=mydb user=myuser password=secret
```

### `certo.toml`'s `[database]` section

```toml
[database]
migrations = "db/migrations/"   # where `certo db migrate` looks for migration files
schema     = "..."              # reserved for future toolchain use — not read today
seeds      = "..."              # reserved for future toolchain use — not read today
```

Only `migrations` actually drives anything right now (§9). Don't be misled
by `schema` and `seeds` being valid, parsed TOML keys — the connection
string always comes from `DATABASE_URL`, not from `certo.toml`.

Live schema cross-checking is a separate, opt-in switch under `[features]`,
not `[database]`:

```toml
[features]
schema-sync = true
```

With this on, `certo build`/`certo check` connects to `DATABASE_URL` and
verifies every type that has `impl DbRow for X {}` (§6) against the live
`information_schema` — table exists, columns exist, types and nullability
match. It's off by default, so an ordinary build never touches the network.

---

## 3. Raw Queries and Execution

These are the functions you'll use the most, and the ones every other layer
in this guide is ultimately built on. All of them need `import Stdlib.Db`.

```certo
dbExec(conn: Int, sql: Text, params: List<Text>): Int
    // rows affected, or -1 on error

dbQuery(conn: Int, sql: Text, params: List<Text>): List<List<Text?>>
    // outer list = rows, inner list = columns; SQL NULL becomes None
    // returns [] on error (not a Result — check dbError(conn) if a query
    // you expected rows from comes back empty)

dbQueryTyped<T>(conn: Int, sql: Text, params: List<Text>,
                mapper: fn(List<Text?>): T): List<T>
    // runs dbQuery, then applies `mapper` to every row

dbQueryRow(conn: Int, sql: Text, params: List<Text>): List<Text?>?
    // first row, or None if the query returned zero rows

dbQueryOne(conn: Int, sql: Text): Text?
    // first column of the first row, or None — NOTE: no params list

dbColumns(conn: Int, sql: Text): List<Text>
    // column names of the result set — NOTE: also no params list

dbStream(conn: Int, sql: Text, params: List<Text>,
         handler: fn(List<Text?>): Unit): Int
    // server-side cursor, fetches 100 rows per round trip, calls
    // `handler` once per row; returns the total row count

dbNull(): Text
    // the sentinel to put in a `params` slot to bind SQL NULL
```

A complete, working example — connect, create a table, insert with
parameters, query it back, and print the results:

```certo
module DbUsers

import Stdlib.Core
import Stdlib.Db

fn main(): Unit [io] = {
    val conn = dbConnect(arg(1) ?? "host=localhost dbname=postgres user=postgres")
    if conn == 0 then {
        eprintln(f"could not connect: {dbError(0)}")
    } else {
        dbExec(conn,
            "CREATE TABLE IF NOT EXISTS users (" ++
            "  id    SERIAL PRIMARY KEY," ++
            "  name  TEXT NOT NULL," ++
            "  email TEXT NOT NULL UNIQUE," ++
            "  age   INT  NOT NULL" ++
            ")", [])

        dbExec(conn,
            "INSERT INTO users(name, email, age) VALUES ($1, $2, $3)" ++
            " ON CONFLICT (email) DO NOTHING",
            ["Alice", "alice@example.com", "30"])

        val rows = dbQuery(conn, "SELECT name, email, age FROM users ORDER BY id", [])
        for row in rows {
            val name  = List.getOrPanic(row, 0) ?? ""
            val email = List.getOrPanic(row, 1) ?? ""
            val age   = List.getOrPanic(row, 2) ?? ""
            println(f"{name} <{email}>, age {age}")
        }

        dbClose(conn)
    }
}
```

A couple of things worth internalizing from that example:

- Every cell in a `dbQuery` row is `Text?` — even the integer `age` column
  comes back as `Some("30")`, not `Some(30)`. You convert with `parseInt`,
  `parseFloat`, and so on wherever you need the real type.
- `List.getOrPanic(row, 0) ?? ""` is the standard idiom for "give me this
  cell as plain `Text`, treating SQL `NULL` as an empty string" — `??` here
  is unwrapping the `Option` that `getOrPanic`'s own cell type carries
  (`Text?`), not the `getOrPanic` call itself, which panics on an
  out-of-range *index* rather than returning `Option`.
- To bind `NULL` in a parameter list, use `dbNull()` — since every
  parameter is `Text`, there's no other spelling for it:
  ```certo
  dbExec(conn, "UPDATE users SET age = $1 WHERE id = $2",
      [if newAge == null then dbNull() else intToText(newAge ?? 0), intToText(id)])
  ```

If a query's result set structure — not just its row values — needs
inspecting dynamically, `dbColumns` gives you the column names:

```certo
val cols = dbColumns(conn, "SELECT id, name, email, age FROM users")
println(List.fold(cols, "") { acc, col => if acc == "" then col else acc ++ " | " ++ col })
```

And when a script needs to run a multi-statement `.sql` file verbatim (say,
a hand-maintained seed script) — `dbExec`/`dbQuery` use `PQexecParams`,
which rejects multi-statement SQL — reach for `dbRunScript` instead:

```certo
dbRunScript(conn, "CREATE TABLE a (id INT); CREATE TABLE b (id INT);")   // returns 1/0
```

or, if you need to inspect what happened:

```certo
val result = dbRunScriptResult(conn, sqlScript)
if DbResult.ok(result) then {
    println(f"columns: {intToText(List.len(DbResult.columns(result)))}")
} else {
    eprintln(f"script failed: {DbResult.error(result)}")
}
```

---

## 4. SQL Injection Protection

Certo's compiler rejects, as a **hard compile error**, an f-string with a
live `{ }` interpolation written directly as the `sql` argument to any raw
query/exec function:

```certo
dbQuery(conn, f"SELECT * FROM users WHERE email = '{email}'", [])
```

```
error[E0216]: an interpolated f-string was passed directly as the `sql`
argument to `dbQuery`
```

The fix is always the same — move the value into the `params` list and
reference it by position:

```certo
dbQuery(conn, "SELECT * FROM users WHERE email = $1", [email])
```

Two things worth knowing about the limits of this check:

- It's purely **syntactic**, checked on the literal expression written at
  the call site. Building the string one line earlier
  (`val sql = f"..."; dbQuery(conn, sql, [])`) isn't caught — there's no
  taint-tracking, only a direct-argument check. Treat E0216 as a guardrail
  against the most common mistake, not a substitute for always
  parameterizing.
- `dbQueryOne` and `dbColumns` take **no `params` list at all** — they run
  their SQL via `PQexec`, not `PQexecParams`. E0216 still catches an
  interpolated f-string passed to either of them, but there's no
  parameterized alternative to switch to; don't put untrusted values in the
  SQL text you hand to these two functions.

---

## 5. Transactions

Two layers here too — pick whichever fits the shape of your code.

**Manual, statement-level:**

```certo
dbBegin(conn)
dbExec(conn, "DELETE FROM users WHERE name = $1", ["Bob"])
dbRollback(conn)   // Bob is back
```

`dbBegin`, `dbCommit`, and `dbRollback` issue exactly the SQL statements
their names suggest. Simple, and easy to reason about for a short script.

**Structured, `Result`-driven:**

```certo
withTransaction<T,E>(conn: Int, body: fn(): Result<T,E>): Result<T,E>
```

`withTransaction` commits automatically when `body()` returns `Ok`, and
rolls back when it returns `Err` — you never write the commit/rollback call
yourself:

```certo
val outcome: Result<Unit, Text> = withTransaction(conn, () => {
    val affected = dbExec(conn, "UPDATE accounts SET balance = balance - $1 WHERE id = $2",
        [intToText(amount), intToText(fromId)])
    if affected == 0 then Err("account not found") else Ok(unit)
})
```

It's genuinely nesting-aware: call `withTransaction` again while already
inside one, and it transparently switches to `SAVEPOINT` /
`RELEASE SAVEPOINT` / `ROLLBACK TO SAVEPOINT` instead of a second
`BEGIN`/`COMMIT`, so a failure in an inner block doesn't take down an outer
transaction that's still in progress.

For the common "open a connection, do one unit of work, always close it"
shape, `withConnection` saves you the `dbClose` bookkeeping entirely — it
closes the connection whether `body` succeeds or fails:

```certo
withConnection<T,E>(url: Text, body: fn(Int): Result<T,E>): Result<T,E>
```

```certo
val result = withConnection(dsn) { conn => {
    dbExec(conn, "INSERT INTO events(kind) VALUES ($1)", ["signup"])
    Ok(unit)
} }
```

---

## 6. Mapping Rows to Types

There's no special schema-declaration syntax in Certo — a "database-backed
type" is just an ordinary `type`, marked with an empty `DbRow` impl:

```certo
type Orders = {
    id: Int
    customerId: Int
    status: Text
    total: Int
}
impl DbRow for Orders {}
```

`impl DbRow for Orders {}` doesn't add any methods — it's a **marker**.
Two things look for it:

- `[features] schema-sync = true` (§2) scans for every `DbRow`-marked type
  and cross-checks it against the live database at compile time.
- The `Query`/`Mutation` DSL (§7, §8) recognizes `"Orders"` as a table name
  by matching it against a `type Orders = { ... }` declaration in the same
  module — the type's field list *is* the schema, as far as the DSL is
  concerned.

Since every raw query cell comes back as `Text?`, you write one small
mapper function per type to turn a row into a value:

```certo
fn ordersFromRow(row: List<Text?>): Orders = Orders {
    id:         parseInt(List.getOrPanic(row, 0) ?? "") ?? 0,
    customerId: parseInt(List.getOrPanic(row, 1) ?? "") ?? 0,
    status:     List.getOrPanic(row, 2) ?? "",
    total:      parseInt(List.getOrPanic(row, 3) ?? "") ?? 0,
}
```

Column order in the mapper must match the column order your query actually
returns — the compiler can't check that for a hand-written `dbQuery`/
`dbQueryTyped` call (it *can* for the `Query` DSL, since it generates the
`SELECT` list itself). Used together:

```certo
val orders: List<Orders> = dbQueryTyped(conn,
    "SELECT id, customerId, status, total FROM orders ORDER BY id", [],
    ordersFromRow)
```

If you'd rather not hand-write this for an existing database, `certo db
pull` (§9) generates exactly this shape — `type` + `impl DbRow` + a
`FromRow` mapper + a handful of `find`/`insert` helpers — straight from
your live schema.

---

## 7. The Typed Query DSL

`import Stdlib.DbQuery` gets you `Query`, a small pipeline builder that the
compiler checks against your `type` declarations *before* it ever runs —
an unknown table, an unknown column, or a typo'd operator is a compile
error, not a runtime surprise.

```certo
fn pendingOrders(conn: Int): List<Orders> =
    Query.from("Orders")
    |> Query.filter("status", "=", "pending")
    |> Query.orderBy("id", "desc")
    |> Query.limit(20)
    |> Query.list(conn, ordersFromRow)
```

The load-bearing rule to remember: **table names, column names, operators,
sort directions, aggregate function names, and aliases must all be string
literals written directly at the call site** — never a variable. That's
what lets the compiler verify each one statically. Values you're filtering
*by* are the opposite — `Query.filter("status", "=", pendingStatus)` is
fine and always travels as a bound `$N` parameter, never interpolated into
SQL text.

The full builder surface:

```certo
Query.from(table)                                   Query.fromAs(table, alias)
Query.filter(q, column, op, value)                   // op: "=" "!=" "<" "<=" ">" ">=" "like"
Query.orderBy(q, column, dir)                        // dir: "asc" "desc"
Query.limit(q, n)                                    Query.offset(q, n)
Query.join(q, table, leftCol, rightCol)               Query.leftJoin(q, table, leftCol, rightCol)
Query.joinAs(q, table, alias, leftCol, rightCol)      Query.leftJoinAs(q, table, alias, leftCol, rightCol)
Query.groupBy(q, column)
Query.aggregate(q, aggFn, column, alias)              // aggFn: "count" "sum" "avg" "min" "max"
Query.having(q, aggFn, column, op, value)
Query.sql(q)                    // the generated SQL, as text — handy for debugging
Query.count(q, conn)
Query.list(q, conn, mapper)                Query.first(q, conn, mapper)
Query.groupedList(q, conn, mapper)         // only after groupBy/aggregate
Query.sum/avg/min/max(q, column, conn)     // scalar aggregate over an ungrouped query
```

Join columns must be **qualified**, `"Table.column"`, on both sides:

```certo
fn ordersWithCustomerName(conn: Int): List<Orders> =
    Query.from("Orders")
    |> Query.join("Customers", "Orders.customerId", "Customers.id")
    |> Query.filter("Customers.name", "=", "Alice")
    |> Query.list(conn, ordersFromRow)
```

A self-join needs an explicit alias — joining the same table twice without
one is a compile error, not silently wrong SQL:

```certo
fn managerChain(conn: Int): List<Text> =
    Query.fromAs("Employees", "e")
    |> Query.joinAs("Employees", "m", "e.managerId", "m.id")
    |> Query.list(conn, employeeMapper)
```

Grouped queries must be read with `Query.groupedList`, not `Query.list` —
reading a grouped query with the wrong terminal call is a compile error:

```certo
fn bigSpenders(conn: Int): List<Text> =
    Query.from("Orders")
    |> Query.groupBy("customerId")
    |> Query.aggregate("sum", "total", "totalSpend")
    |> Query.having("sum", "total", ">", "1000")
    |> Query.groupedList(conn, spendMapper)
```

---

## 8. The Mutation DSL

`import Stdlib.DbMutation` gets you `Mutation`, the write-side counterpart
to `Query` — same static-checking discipline (literal table/column names,
values always bound as parameters):

```certo
Mutation.insertInto(table)      Mutation.updateTable(table)     Mutation.deleteFrom(table)
Mutation.insertMany(table, columns: List<Text>)
Mutation.set(m, column, value)          // insert / update only
Mutation.filter(m, column, op, value)   // update / delete only
Mutation.onConflict(m, column)          // insert only — upsert via ON CONFLICT DO UPDATE
Mutation.addRow(m, values: List<Text>)  // insertMany only
Mutation.run(m, conn): Int              // rows affected
```

```certo
fn placeOrder(conn: Int, customerId: Int, total: Int): Int =
    Mutation.insertInto("Orders")
    |> Mutation.set("customerId", intToText(customerId))
    |> Mutation.set("status", "pending")
    |> Mutation.set("total", intToText(total))
    |> Mutation.run(conn)

fn markShipped(conn: Int, orderId: Int): Int =
    Mutation.updateTable("Orders")
    |> Mutation.set("status", "shipped")
    |> Mutation.filter("id", "=", intToText(orderId))
    |> Mutation.run(conn)

fn cancelOrder(conn: Int, orderId: Int): Int =
    Mutation.deleteFrom("Orders")
    |> Mutation.filter("id", "=", intToText(orderId))
    |> Mutation.run(conn)
```

An upsert:

```certo
Mutation.insertInto("Prices")
|> Mutation.set("productId", intToText(productId))
|> Mutation.set("amount", intToText(amount))
|> Mutation.onConflict("productId")
|> Mutation.run(conn)
```

And a batch insert — one round trip for many rows, with the row width
checked against the column list you declared:

```certo
Mutation.insertMany("Products", ["name", "price"])
|> Mutation.addRow(["Widget", "9.99"])
|> Mutation.addRow(["Gadget", "19.99"])
|> Mutation.run(conn)
```

---

## 9. Migrations

Migration files are ordinary `.cto` source files, one per migration, kept
in whatever directory `certo.toml`'s `[database] migrations` points at
(`db/migrations/` by default in a project scaffolded with `certo new`).

```certo
module Migration

migration "create_users" {
    up {
        createTable users {
            id: UUID primaryKey,
            email: Text unique nullable,
            age: Int default 0
        }
    }
    down {
        dropTable users
    }
}
```

Columns are `NOT NULL` by default — add `nullable` to allow `NULL`. A
later migration can alter that table:

```certo
migration "add_author_and_drop_legacy" {
    up {
        alterTable posts {
            addColumn authorId: UUID
            dropColumn legacy
            foreignKey authorId references users onDelete cascade
        }
    }
    down {
        alterTable posts { dropColumn authorId }
    }
}
```

Operations available inside `up`/`down`: `createTable`, `alterTable`
(`addColumn` / `dropColumn` / `foreignKey ... references ... onDelete
cascade|setNull|restrict|noAction`), `dropTable`, `createIndex ... on
table [col, ...]`, `dropIndex`, and `rawSql "..."` as an escape hatch for
anything the structured grammar doesn't cover.

The CLI:

```bash
certo db create add_users_table   # scaffold a new, empty migration file
certo db migrate                  # apply every pending migration, in file order
certo db migrate --dry-run        # print the DDL without running it
certo db rollback                 # roll back the most recent migration
certo db rollback 3               # roll back the 3 most recent
certo db status                   # show applied vs. pending migrations
```

`certo db migrate`/`rollback` generate the SQL, then shell out to a real
`psql` on `PATH` with `--single-transaction --set ON_ERROR_STOP=1` — a
whole batch of pending migrations applies as one atomic transaction, and
the local migration-state file is only updated after `psql` succeeds. Like
everything else in this guide, they read the connection from
`DATABASE_URL` (env var or `.env`).

Two more commands close the loop between a live database and your `.cto`
source:

```bash
certo db pull [-o db/schema.cto]   # introspect the live DB, write matching
                                    # type + impl DbRow + FromRow + helpers
certo db diff db/schema.cto        # compare that file against the live DB,
                                    # report drift
```

`certo db pull`'s output is real, working code you can use as-is or as a
starting point — it's exactly the pattern shown in §6, generated
automatically, one `type`/mapper/helper set per table.

Building your whole project (`certo build`/`certo check`) also cross-checks
every migration against your `type` declarations — a `createTable orders
{ ... }` with no matching `type Orders = { ... }` anywhere in your project
is a compile error, not something you find out at migration time. Keep the
two in sync (running `certo db pull` after a migration is the easiest way).

---

## 10. Testing Database Code

`certo-test` has a dedicated block, `dbTest`, that runs its body inside a
transaction which is **always rolled back** — your assertions can insert,
update, and delete freely, and the database is left exactly as it was
before the test ran:

```certo
dbTest "inserting a user increases the row count" {
    val before = dbQueryOne(conn, "SELECT COUNT(*) FROM users") ?? "0"
    dbExec(conn, "INSERT INTO users(name, email, age) VALUES ($1, $2, $3)",
        ["Test", "test@example.com", "1"])
    val after = dbQueryOne(conn, "SELECT COUNT(*) FROM users") ?? "0"
    assert(after != before, "row count should have changed")
}
```

A `conn: Int` is provided automatically inside every `dbTest` block — you
don't call `dbConnect` yourself. Behind the scenes, `certo test` expands
each `dbTest` into a real function that reads `DATABASE_URL`, opens a
connection, issues `BEGIN`, runs your body, and issues `ROLLBACK` — never
`COMMIT`. `certo test` auto-detects when a test file uses `dbTest` (or
imports the DB stdlib) and links `libpq` for that run; you don't need to
configure anything extra. As with every other command in this guide,
`DATABASE_URL` must point at a real, reachable PostgreSQL instance — a
missing `DATABASE_URL` fails fast with a clear message rather than
silently skipping the test.

---

## 11. Exploring Live Data with the REPL

```bash
certo repl --connect "host=localhost dbname=mydb user=myuser"
```

This makes a function `conn(): Int` available in the session — call it
to get a fresh connection:

```
> val rows = dbQuery(conn(), "select id, email from users limit 5", [])
> for r in rows { println(List.getOrPanic(r, 1) ?? "") }
alice@example.com
bob@example.com
...
```

`conn()` is a *function call*, not a bare variable, deliberately — each
REPL turn recompiles and re-runs your whole accumulated session as a fresh
subprocess (there's no persistent interpreter state between turns), so
"the connection" can only ever mean "connect again, right now." Without
`--connect`, the database stdlib isn't linked into the REPL session at all,
which keeps ordinary non-database REPL turns fast to compile.

This is the fastest way to sanity-check a query, inspect a table's actual
contents, or try out a `Query`/`Mutation` pipeline before committing it to
a source file.

---

## 12. A Small Worked Example

Putting several pieces from this guide together — a tiny order-tracking
program using the typed DSL, a transaction, and a raw report query:

```certo
module OrderTracker

import Stdlib.Core
import Stdlib.Db
import Stdlib.DbQuery
import Stdlib.DbMutation

type Orders = {
    id: Int
    customerId: Int
    status: Text
    total: Int
}
impl DbRow for Orders {}

fn ordersFromRow(row: List<Text?>): Orders = Orders {
    id:         parseInt(List.getOrPanic(row, 0) ?? "") ?? 0,
    customerId: parseInt(List.getOrPanic(row, 1) ?? "") ?? 0,
    status:     List.getOrPanic(row, 2) ?? "",
    total:      parseInt(List.getOrPanic(row, 3) ?? "") ?? 0,
}

fn placeOrder(conn: Int, customerId: Int, total: Int): Int =
    Mutation.insertInto("Orders")
    |> Mutation.set("customerId", intToText(customerId))
    |> Mutation.set("status", "pending")
    |> Mutation.set("total", intToText(total))
    |> Mutation.run(conn)

fn shipOldestPending(conn: Int): Result<Unit, Text> =
    withTransaction(conn, () => {
        val oldest = Query.from("Orders")
            |> Query.filter("status", "=", "pending")
            |> Query.orderBy("id", "asc")
            |> Query.limit(1)
            |> Query.first(conn, ordersFromRow)
        match oldest {
            None => Err("no pending orders")
            Some(order) => {
                Mutation.updateTable("Orders")
                |> Mutation.set("status", "shipped")
                |> Mutation.filter("id", "=", intToText(order.id))
                |> Mutation.run(conn)
                Ok(unit)
            }
        }
    })

fn totalRevenue(conn: Int): Text =
    dbQueryOne(conn, "SELECT COALESCE(SUM(total), 0) FROM orders WHERE status = 'shipped'") ?? "0"

fn main(): Unit [io] = {
    val conn = dbConnect(getEnv("DATABASE_URL") ?? panic("DATABASE_URL is not set"))
    if conn == 0 then {
        eprintln(f"could not connect: {dbError(0)}")
    } else {
        placeOrder(conn, 1, 4200)
        placeOrder(conn, 2, 1500)

        match shipOldestPending(conn) {
            Ok(_)  => println("shipped the oldest pending order")
            Err(e) => println(f"nothing to ship: {e}")
        }

        println(f"revenue so far: {totalRevenue(conn)}")
        dbClose(conn)
    }
}
```

This uses raw `dbQueryOne` for the one-off aggregate report (§3), the
`Query`/`Mutation` DSL for everything shaped like ordinary application
reads and writes (§7, §8), and `withTransaction` (§5) to keep the
"find the oldest pending order, then ship it" read-then-write sequence
atomic. That mix — reach for the DSL by default, drop to raw SQL for
reports and anything the DSL doesn't model — is the idiomatic way to write
database code in Certo.

---

## 13. Limitations and Where to Go Next

Worth knowing before you build something real:

- **PostgreSQL only.** No other database backend exists.
- **No connection pool, no async driver.** Every `dbConnect` is one
  blocking libpq connection; concurrency is your program's problem, not
  the stdlib's.
- **Most raw query functions swallow SQL errors into an empty result**
  (`dbQuery` → `[]`, `dbQueryRow`/`dbQueryOne` → `None`) rather than
  signalling failure through the return type. Only `dbExec` (`-1`) and
  `dbRunScriptResult` (`DbResult`) surface an explicit error. If a query
  you expected rows from comes back empty, check `dbError(conn)`.
- **E0216 is syntactic, not a taint tracker** (§4) — it only catches an
  f-string interpolation written directly at the call site.
- **No computed table/column names** in the `Query`/`Mutation` DSL — every
  such argument must be a string literal, by design (§7).
- **No dot-call syntax anywhere in the language** — it's always
  `List.getOrPanic(row, 0)`, never `row.getOrPanic(0)`. This isn't specific
  to the database stdlib; it's how every module in Certo works.

From here, the rest of the standard library ([docs/STDLIB-QUICKREF.md](STDLIB-QUICKREF.md))
and the full compile-error reference ([docs/ERROR-REFERENCE.md](ERROR-REFERENCE.md),
error codes E0216 and E0500–E0526 cover everything in this guide) are the
next places to look. `examples/db_users.cto` and `examples/db/` in the
repository root are real, runnable programs you can build with
`certo run` against your own PostgreSQL instance.
