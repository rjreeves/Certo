# Certo Compiler Error Reference

Every error and warning code the compiler can emit, with the cause,
the fix, and a minimal reproduction.

---

## Error ranges at a glance

| Range | Category |
|---|---|
| [E0100–E0102](#e0100-e0102-name-resolution) | Name resolution |
| [E0200–E0215](#e0200-e0215-type-errors) | Type errors |
| [E0216](#e0216-sql-injection-risk) | SQL injection risk |
| [E0300–E0306](#e0300-e0306-trait-errors) | Trait / impl errors |
| [E0400–E0404](#e0400-e0404-effect-errors) | Effect annotations |
| [E0500–E0507](#e0500-e0507-migration--schema-errors) | Migration & schema |
| [E0508–E0526](#e0508-e0526-db-query-dsl-errors) | DB query DSL |
| [E0600–E0601](#e0600-e0601-hir-lowering-errors) | HIR lowering |
| [E0700–E0702](#e0700-e0702-validator-rule-errors) | Validator rules |
| [E0708–E0709](#e0708-e0709-temporal--age-errors) | Temporal / `.age` |
| [E0710](#e0710-unsupported-keynumeric-projection-type) | `sortBy`/`minBy`/`maxBy`/`sumBy` projections |
| [L001–L005](#l001-l005-lint-warnings) | Lint warnings |

---

## E0100–E0102  Name resolution

### E0100  Undefined identifier

```
error[E0100]: undefined name `processOrders`
  --> src/main.cto:12:5
   |
12 |     processOrders(orders)
   |     ^^^^^^^^^^^^^ not defined in this scope
   |
   = note: check the spelling; stdlib functions are accessed as
           `Module.function`, e.g. `Text.len`
```

**Cause:** A name is used that was never declared in the current scope or any
enclosing scope.

**Common causes and fixes:**

| Symptom | Fix |
|---|---|
| Missing `import` | Add `import Stdlib.Http` etc. |
| Typo in function/variable name | Check spelling |
| Wrong casing (`text.len` instead of `Text.len`) | Certo names are case-sensitive |
| Used before declaration | Move the definition above the use |
| `none` / `some` / `ok` / `err` (lowercase) | Use `None` / `Some` / `Ok` / `Err` |
| `list` instead of `List.empty` | `List.empty()` or `[]` for empty list |

---

### E0101  Ambiguous import

```
error[E0101]: ambiguous import — `format` is imported more than once
  --> src/main.cto:3:8
```

**Cause:** Two `import` statements bring a name with the same short form into
scope.

**Fix:** Use the fully qualified name (`Module.name`) instead of the short form
for one or both usages.

---

### E0102  Duplicate definition

```
error[E0102]: duplicate definition of `process` in this scope
  --> src/orders.cto:27:4
```

**Cause:** Two declarations (functions, types, or bindings) share the same name
in the same scope.

**Fix:** Rename one of them, or move one to a different scope or module.

---

## E0200–E0215  Type errors

### E0200  Type mismatch

```
error[E0200]: type mismatch: expected `Text`, found `Int`
  --> src/main.cto:8:20
   |
 8 |     println(userId)
   |             ^^^^^^ this has type `Int`
   |
   = note: use `intToText(n)` to convert an Int to Text
```

**Cause:** An expression has a different type from what is expected at that
position.

**Context-specific hints the compiler adds:**

| Situation | Hint |
|---|---|
| `Int` where `Text` expected | Use `intToText(n)` |
| `Text` where `Int` expected | Use `parseInt(s)` (returns `Int?`) |
| `Float` where `Int` expected | Use `floatToInt(f)` |
| `Int` where `Float` expected | Use `intToFloat(n)` |
| `Float` where `Text` expected | Use `floatToText(f)` |
| `T?` where `T` expected | Unwrap with `match` or `??` |
| `Result<T, E>` where `T` expected | Handle with `match` or `?` |
| Non-bool in condition | Conditions must be `Bool` |
| `Unit` used as a value | The function returns nothing; remove the assignment |
| Function passed where value expected | Did you mean to call it? |
| Value passed where function expected | Expected a function, got a plain value |

**Common examples:**

```
// Wrong — println expects Text
println(42)
// Fix
println(intToText(42))

// Wrong — optional not unwrapped
val user: User = findUser(id)   // findUser returns User?
// Fix
match findUser(id) {
    Some(u) => u
    None    => return Err("not found")
}

// Wrong — Result not handled
val n: Int = parseInt("42")    // parseInt returns Int?
// Fix
val n: Int = parseInt("42") ?? 0
```

---

### E0201  Cannot unify

```
error[E0201]: cannot unify `List<Int>` with `List<Text>`
  --> src/main.cto:14:15
   |
14 |     val xs = concat([1, 2], ["a", "b"])
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^
   |               incompatible types `List<Int>` and `List<Text>`
   |
   = note: these two types must match but they have different shapes
```

**Cause:** Two type expressions are unified during inference but have
incompatible structures. Similar to E0200 but typically arises from type
variables being constrained in conflicting directions.

**Fix:** Ensure both sides of the expression have the same type. Often caused
by mixing element types inside a list or map literal, or passing arguments
of the wrong type to a generic function.

---

### E0202  Occurs check / infinite type

```
error[E0202]: infinite type: a type variable appears within its
              own inferred type `List<?t0>`
  --> src/main.cto:5:5
   |
   = note: this usually means a recursive type alias without a base case
   = note: if you meant a recursive function, add an explicit return type
```

**Cause:** The type inferencer tried to construct an infinite type — a type
that contains itself. This almost always means a recursive function with no
explicit return type annotation, where the inferencer cannot determine where
the recursion terminates.

**Fix:** Add an explicit return type annotation to the recursive function:

```
// Wrong
fn repeat(s: Text, n: Int) =
    if n == 0 then "" else s ++ repeat(s, n - 1)

// Fix
fn repeat(s: Text, n: Int): Text =
    if n == 0 then "" else s ++ repeat(s, n - 1)
```

---

### E0203  Recursive function needs return type

```
error[E0203]: recursive function `factorial` needs an explicit return type
  --> src/math.cto:3:4
   |
 3 | fn factorial(n: Int) = if n <= 1 then 1 else n * factorial(n - 1)
   |    ^^^^^^^^^  return type required here
   |
   = note: add `: ReturnType` after the parameter list — for example:
             fn factorial(n: Int): Int = ...
```

**Cause:** A function calls itself (directly or indirectly) but has no
explicit return type. The type inferencer needs the annotation to break the
cycle.

**Fix:** Add `: ReturnType` between the parameter list and `=`:

```
fn factorial(n: Int): Int =
    if n <= 1 then 1 else n * factorial(n - 1)

fn fib(n: Int): Int =
    if n <= 1 then n else fib(n - 1) + fib(n - 2)
```

---

### E0204  Wrong number of arguments

```
error[E0204]: wrong number of arguments: expected 2, found 3
  --> src/main.cto:10:5
   |
10 |     add(1, 2, 3)
   |     ^^^^^^^^^^^^ this call passes 3 argument(s)
   |
   = note: remove 1 extra argument(s)
```

**Cause:** A function call passes a different number of arguments than the
function's parameter list declares.

**Fix:** Match the argument count to the declaration. Use `certo check -v` to
see the expected signature.

---

### E0205  Unknown field

```
error[E0205]: no field `userName` on type `User`
  --> src/main.cto:15:12
   |
15 |     user.userName
   |          ^^^^^^^^ `User` has no field named `userName`
```

**Cause:** Field access with a name that does not exist on the type.

**Common causes:**

| Symptom | Fix |
|---|---|
| Typo (`userName` vs `name`) | Check the field name spelling |
| Wrong type (accessing `User` field on an `Order`) | Verify the variable type |
| Accessing a module function as a field (`text.len`) | Use function syntax: `Text.len(text)` |
| Tuple access with `.0` | Tuple integer indexing is not supported; use `match (a, b) { (x, y) => x }` |

---

### E0206  Unbound name (belt-and-suspenders)

Same message as E0100 but emitted by the type-checker rather than the
resolver. In practice, if you see this, it is a sign that the resolver did
not catch the undefined name first. The fix is the same as E0100: declare or
import the name.

---

### E0210  `extern` call outside `unsafe`

```
error[E0210]: call to extern function `sqlite3_open` must be inside an
`unsafe { }` block
  --> src/ffi.cto:9:5
```

**Cause:** A call to an `extern "C"` (FFI) function appears outside an
`unsafe { }` block.

**Fix:** Wrap the call: `unsafe { sqlite3_open(...) }`.

---

### E0211  Non-displayable interpolation

```
error[E0211]: cannot interpolate a value of type `User` into a string
  --> src/main.cto:12:20
```

**Cause:** An f-string interpolation (`f"... {expr} ..."`) holds a value
whose type has no text representation.

**Fix:** Only `Int`, `Float`, `Bool`, `Decimal`, and `Text` can be
interpolated directly — convert the value first (e.g. call a function that
produces `Text` from it).

---

### E0212  Missing row-polymorphism field

```
error[E0212]: `Order` does not satisfy the row bound
  --> src/main.cto:20:10
    = missing field `email: Text`
```

**Cause:** An argument passed for a row-polymorphism-bounded type parameter
(`R: { name: Text }`) is missing a field the bound requires.

**Fix:** Add the missing field, or pass a value of a type that already has
it.

---

### E0213  Non-exhaustive match

```
error[E0213]: match on `Shape` is not exhaustive
  --> src/main.cto:18:5
    = missing: Triangle
```

**Cause:** A `match` does not cover every possible value of the
scrutinee's type. Fully checked for `Bool`, `Option`, `Result`, and
user-declared sum types — for everything else (`Int`, `Text`, records,
etc.) a trailing `_` catch-all arm is required, since there's no finite
set of cases to enumerate.

**Fix:** Add the missing arm(s), or a trailing `_ => ...` arm to cover the
rest.

---

### E0214  Private constructor called outside its `impl`

```
error[E0214]: constructor `Email` is private
  --> src/main.cto:14:16
```

**Cause:** A `type X = priv X(...)` smart-constructor's raw constructor was
called from outside an `impl X { ... }` block for the same type.

**Fix:** Call it only from within `impl X { ... }` — typically via a
validating factory function such as `X.new(...)`.

---

### E0215  Secret exposed to a sensitive sink

```
error[E0215]: `Text` is not Loggable/Serializable
  --> src/main.cto:22:13
    = passed to `println`, which would expose it
```

**Cause:** A value whose type structurally contains `Secret<_>` (however
deeply wrapped — inside an `Option`, `List`, record field, etc.) was passed
to a logging/serialization sink (`println`, `print`, `eprint`,
`Json.stringify`).

**Fix:** Call `.expose()` on the `Secret` first if you genuinely need its
raw value at that call site — this is a deliberate, visible opt-out, not
something to reach for by default.

---

## E0216  SQL injection risk

### E0216  Interpolated f-string passed as raw SQL

```
error[E0216]: an interpolated f-string was passed directly as the `sql`
argument to `dbQuery` — use `?` placeholders and pass values via `params`
instead
  --> src/users.cto:14:14
```

**Cause:** An f-string with a live `{ }` interpolation was passed directly
as the `sql` argument of a raw-SQL sink (`dbQuery`, `dbExec`,
`dbQueryTyped`, `dbQueryRow`, `dbQueryOne`, `dbColumns`, `dbStream`,
`dbRunScript`, `dbRunScriptResult`). Interpolating untrusted or
user-derived values straight into SQL text is a SQL-injection
vulnerability — the whole reason these functions accept a separate
`params` list.

**Fix:** Use `?` placeholders in the SQL text and pass the values through
`params` instead, where the underlying driver parameterizes them safely:

```certo
// Before (rejected):
dbQuery(conn, f"SELECT * FROM users WHERE email = '{email}'", [])

// After:
dbQuery(conn, "SELECT * FROM users WHERE email = ?", [email])
```

This check is syntactic: it only catches an f-string written directly at
the call site. Building the SQL text earlier (e.g. `let sql = f"..."`)
and passing the variable in is not currently caught — write raw SQL
inline, or use the parameterized fluent query builder (`db.query(...)`),
which is safe by construction either way.

---

## E0300–E0306  Trait errors

### E0300  Unknown method in impl

```
error[E0300]: method `serialize` is not declared in trait `Display`
  --> src/domain.cto:22:5
```

**Cause:** An `impl` block contains a method that the trait does not declare.

**Fix:** Remove the extra method from the `impl`, or add it to the `trait`
declaration.

---

### E0301  Missing required method

```
error[E0301]: impl is missing method `display` required by `Display`
  --> src/domain.cto:20:1
```

**Cause:** An `impl` block does not implement all methods declared by the
trait.

**Fix:** Add the missing method to the `impl` block with the correct signature.

---

### E0302  Wrong parameter count in impl method

```
error[E0302]: method `display` expects 1 parameter(s), impl has 2
  --> src/domain.cto:21:5
```

**Cause:** An impl method has a different number of parameters than the trait
declaration.

**Fix:** Match the parameter count exactly to the trait signature.

---

### E0303  Return type mismatch in impl

```
error[E0303]: method `display` return type mismatch — trait `Text`, impl `Int`
  --> src/domain.cto:21:5
```

**Cause:** The return type of an impl method differs from the return type
declared in the trait.

**Fix:** Change the impl method's return type (or body) to match the trait.

---

### E0304  Parameter type mismatch in impl

```
error[E0304]: method `compare` param 0 type mismatch — trait `Int`, impl `Text`
  --> src/domain.cto:25:5
```

**Cause:** A parameter type in an impl method differs from the corresponding
trait declaration.

**Fix:** Match all parameter types to the trait signature.

---

### E0305  Unsatisfied trait bound

```
error[E0305]: type `MyRecord` does not implement `DbRow`
  --> src/users.cto:8:18
   |
 8 |     dbQueryTyped(conn, sql, params, myMapper)
   |                  ^^^^
   |
   = note: only types generated by `certo db pull` implement `DbRow`;
           add `impl DbRow for MyRecord {}` to suppress this error
```

**Cause:** A generic function requires a type to implement a trait, but no
`impl` block for that trait and type was found.

**Fix:** Either add `impl TraitName for MyType {}` (for marker traits like
`DbRow`), or provide a full implementation for traits with methods.

For `DbRow` specifically: if you wrote your own `type` declaration rather than
using `certo db pull`, add the impl manually:

```
type MyRecord = { id: Int, name: Text }
impl DbRow for MyRecord {}
```

---

### E0306  Duplicate impl

```
error[E0306]: duplicate impl of `Displayable` for `Color`
  --> src/domain.cto:30:1
```

**Cause:** Two `impl` blocks provide the same trait for the same type.

**Fix:** Remove one of the duplicate `impl` blocks, or merge them.

---

## E0400–E0404  Effect errors

Effects are declared in `[...]` after the parameter list. A function must
declare every effect it (or its callees) use.

### E0400  Undeclared effect

```
error[E0400]: function `loadUser` uses effect `io` without declaring it
  --> src/users.cto:5:4
```

**Cause:** The function performs I/O (network, file, DB, console) but its
signature does not include `[io]`.

**Fix:** Add the effect annotation:

```
// Wrong
fn loadUser(id: Int): User = dbQueryRow(conn, sql, [])

// Fix
fn loadUser(id: Int): User [io] = dbQueryRow(conn, sql, [])
```

---

### E0401  Pure function calls impure function

```
error[E0401]: pure function `compute` calls `logResult` which requires `io`
  --> src/math.cto:12:5
```

**Cause:** A function with no effect annotations (i.e., a pure function) calls
a function that requires an effect.

**Fix:** Either add the effect annotation to the caller, or restructure so the
pure function does not perform the side-effectful operation.

---

### E0402  `await` without `[async]`

```
error[E0402]: function `fetch` uses `await` but is not declared [async]
  --> src/api.cto:8:5
```

**Cause:** An `await` expression appears in a function that is not declared
`[async]`.

**Fix:** Add `[async]` to the function signature, or remove the `await`.

---

### E0403  `db.transaction` without `[db.write]`

```
error[E0403]: `db.transaction` in `createOrder` requires [db.write] annotation
  --> src/orders.cto:14:5
```

**Cause:** A `db.transaction` block is used in a function not annotated
`[db.write]`.

**Fix:** Add `[db.write]` to the function's effect list.

---

### E0404  `unsafe` block without `[unsafe]`

```
error[E0404]: `unsafe` block in `lowLevel` requires [unsafe] annotation
  --> src/ffi.cto:6:5
```

**Cause:** An `unsafe` block appears in a function not annotated `[unsafe]`.

**Fix:** Add `[unsafe]` to the function signature.

---

## E0500–E0507  Migration & schema errors

These errors are emitted when the compiler checks that your migration
declarations are consistent with your `type` declarations.

### E0500  Unknown table in migration

```
error[E0500]: migration `add index` references unknown table `orders`
  --> db/migrations/003-add-index.cto:5:9
```

**Cause:** A migration operation references a table that was never created by
any `createTable` operation in any migration (possibly in a different file).

**Fix:** Ensure a `createTable orders { ... }` migration exists and is ordered
before this one.

---

### E0501  Column type mismatch

```
error[E0501]: column `users.age` type mismatch
  --> db/migrations/001-create-users.cto:8:9
   |
   = migration uses `Text`, type declaration has `Int`
   = note: update the migration column type to match the `type` declaration
```

**Cause:** A migration declares a column with a type that differs from the
corresponding field in the `type` declaration.

**Fix:** Make the migration column type match the Certo `type` field:

```
// type declaration
type User = { id: Int, age: Int }

// migration — wrong
migration "create users" {
    up { createTable users { id: Int, age: Text } }  // age should be Int
    down { dropTable users }
}
```

---

### E0502  Foreign key references unknown table

```
error[E0502]: foreign key `orders.userId` references unknown table `users`
  --> db/migrations/002-create-orders.cto:6:9
   |
   = note: the referenced table must be declared as a `type` in this module
```

**Cause:** A `references` clause names a table that has no `type` declaration
in the module.

**Fix:** Declare the referenced table as a `type`, or check the table name for
typos.

---

### E0503  Duplicate migration name

```
error[E0503]: duplicate migration name `create users table`
  --> db/migrations/003-another.cto:1:1
```

**Cause:** Two migration declarations share the same name string.

**Fix:** Give each migration a unique, descriptive name. The name is the
string inside the quotes: `migration "..."`.

---

### E0504  Missing `down` block

```
error[E0504]: migration `add users.role column` has no `down` block
  --> db/migrations/002-add-role.cto:1:1
   |
   = note: add a `down { ... }` block to make this migration reversible
```

**Cause:** A migration that adds or modifies schema (a non-destructive
operation) has no `down` block.

**Fix:** Add a `down` block with the inverse operation:

```
migration "add users.role column" {
    up {
        alterTable users { addColumn role: Text default "member" }
    }
    down {
        alterTable users { dropColumn role }
    }
}
```

---

### E0505  Table not declared as a type

```
error[E0505]: migration `create sessions table` creates table `sessions`
              which is not declared as a `type`
  --> db/migrations/004-sessions.cto:2:9
   |
   = note: add `type sessions { ... }` to your module
```

**Cause:** A `createTable` operation names a table that has no corresponding
`type` declaration in the module.

**Fix:** Add a `type` declaration for the table:

```
type Session = {
    id:        Int,
    userId:    Int,
    token:     Text,
    expiresAt: Int
}

migration "create sessions table" {
    up {
        createTable sessions {
            id:        Int  primary key
            userId:    Int  references users(id)
            token:     Text unique
            expiresAt: Int
        }
    }
    down { dropTable sessions }
}
```

---

### E0506  Column not on type

```
error[E0506]: column `nickname` does not exist on type `User`
  --> db/migrations/005-add-nickname.cto:4:13
   |
   = referenced in migration `add nickname column`
   = note: add field `nickname: Text?` to the `type User` declaration
```

**Cause:** A migration references a column name that does not appear as a
field in the corresponding `type` declaration.

**Fix:** Add the field to the `type` declaration to match the migration.

---

### E0507  Alter/drop before create

```
error[E0507]: migration `drop old index` alters/drops `events` before it was created
  --> db/migrations/003-drop-old-index.cto:3:9
   |
   = note: add a `CreateTable` operation for this table in an earlier migration
```

**Cause:** A migration tries to alter or drop a table that has not been
created by any prior `createTable` migration (according to the order the files
are sorted).

**Fix:** Ensure the `createTable` migration for this table sorts before this
one. Use numeric filename prefixes to control order.

---

## E0508–E0526  DB query DSL errors

### E0508  Unknown table in `Query.from`

```
error[E0508]: `Query.from("Ghost")` — no such table
  --> src/main.cto:5:5
   |
   = note: add `type Ghost { ... }` to this module, or check for a typo
```

**Cause:** `Query.from("Table")` names a table with no corresponding
`type` declaration in the module.

**Fix:** Declare the type, or fix the table-name string literal.

---

### E0509  Unknown column in `.filter`/`.orderBy`

```
error[E0509]: column `nickname` does not exist on `Orders`
  --> src/main.cto:6:24
   |
   = note: add field `nickname: <Type>` to the `type Orders` declaration, or check for a typo
```

**Cause:** `.filter`/`.orderBy` references a column name that isn't a field
on the query's table type.

**Fix:** Check the column name against the `type`'s declared fields.

---

### E0510  Non-literal builder argument

```
error[E0510]: `Query.filter`'s column argument must be a string literal
  --> src/main.cto:7:20
   |
   = note: column/operator/table names must be literal so the compiler can verify them against the schema
```

**Cause:** A `Query`/`Mutation` builder call's table/column/operator/
direction argument is a variable or expression, not a string literal — it
can't be checked against the schema at compile time this way.

**Fix:** Pass a literal string directly at the call site.

---

### E0511  Invalid query operator

```
error[E0511]: `"~="` is not a recognized query operator
  --> src/main.cto:6:34
   |
   = note: valid operators: "=", "!=", "<", "<=", ">", ">=", "like"
```

**Cause:** `.filter`'s operator string isn't one of the recognized
operators (shared between `Query` and `Mutation`).

**Fix:** Use one of the operators listed in the note.

---

### E0512  Invalid sort direction

```
error[E0512]: `"sideways"` is not a valid sort direction
  --> src/main.cto:8:29
   |
   = note: valid directions: "asc", "desc"
```

**Cause:** `.orderBy`'s direction argument isn't `"asc"` or `"desc"`.

**Fix:** Use `"asc"` or `"desc"`.

---

### E0513  Invalid aggregate function

```
error[E0513]: `"median"` is not a recognized aggregate function
  --> src/main.cto:9:23
   |
   = note: valid aggregate functions: "count", "sum", "avg", "min", "max"
```

**Cause:** `.aggregate`/`.having`'s function name isn't one of the
recognized aggregates.

**Fix:** Use one of the functions listed in the note.

---

### E0514  Invalid alias

```
error[E0514]: `"not valid!"` is not a valid alias
  --> src/main.cto:9:38
   |
   = note: aliases must start with a letter or underscore, followed by letters, digits, or underscores
```

**Cause:** An `.aggregate` result alias, or a `.joinAs`/`.leftJoinAs`/
`.fromAs` table alias, isn't a valid identifier.

**Fix:** Use a plain identifier, per the note.

---

### E0515  Ambiguous column across joined tables

```
error[E0515]: column `name` is ambiguous
  --> src/main.cto:10:20
   |
   = present on `Customers`, `Employees`
   = note: qualify it, e.g. "Customers.name"
```

**Cause:** An unqualified column name (e.g. `"name"`, not `"Customers.name"`)
matches a column on more than one table in a joined query.

**Fix:** Qualify the column with its table, as the note suggests.

---

### E0516  Terminal call on a grouped/aggregated query

```
error[E0516]: `Query.list` cannot be used on a grouped/aggregated query
  --> src/main.cto:11:5
   |
   = note: after `.groupBy`/`.aggregate`, use `.groupedList` to run the query
```

**Cause:** `.list`/`.first`/`.count`/a scalar aggregate (`.sum`/`.avg`/
`.min`/`.max`) was called on a query that already has `.groupBy`/
`.aggregate` applied — those change the result shape away from `SELECT *`
or a single scalar.

**Fix:** Use `.groupedList` to read a grouped/aggregated query's results
instead.

---

### E0517  Column table not part of this query

```
error[E0517]: `Ghost` is not the base table or a joined table in this query
  --> src/main.cto:12:15
   |
   = note: add a `.join`/`.leftJoin` on this table first, or check for a typo
```

**Cause:** A `"Table.column"` qualifier names a table that is neither the
query's base table nor one of its joined tables.

**Fix:** Fix the table name, or add the missing `.join`/`.leftJoin`.

---

### E0518  Unqualified join column

```
error[E0518]: join column `"customerId"` must be qualified
  --> src/main.cto:13:29
   |
   = note: write it as "Table.customerId"
```

**Cause:** `.join`/`.leftJoin`'s ON columns must be written as
`"Table.column"`, not a bare column name.

**Fix:** Qualify both sides: `Query.join("Customers", "Orders.customerId",
"Customers.id")`.

---

### E0519  Unknown table in a `Mutation`

```
error[E0519]: `insertInto("Ghost")` — no such table
  --> src/main.cto:6:5
   |
   = note: add `type Ghost { ... }` to this module, or check for a typo
```

**Cause:** `Mutation.insertInto`/`.updateTable`/`.deleteFrom`/
`.insertMany` references a table with no corresponding `type` declaration.

**Fix:** Declare the type, or fix the table-name string literal.

---

### E0520  Mutation method used on the wrong kind

```
error[E0520]: `.filter` cannot be used on an insert
  --> src/main.cto:7:5
```

**Cause:** A `Mutation` method was called on a mutation kind it doesn't
apply to — e.g. `.filter` on an `insertInto`, or `.set` on a `deleteFrom`.
`.set` is valid on insert/update, `.filter` on update/delete, `.onConflict`
on insert (upsert), `.addRow` on `insertMany`.

**Fix:** Remove the call, or use the mutation kind it's actually valid for.

---

### E0521  `.addRow` arity mismatch

```
error[E0521]: `.addRow` has 1 value(s), but `.insertMany` declared 2 column(s)
  --> src/main.cto:8:5
```

**Cause:** `.addRow`'s value list doesn't have the same number of entries
as `.insertMany`'s declared column list (checked only when both are
literal lists).

**Fix:** Make each `.addRow` call supply exactly one value per declared
column, in the same order.

---

### E0522  Live table missing (schema-sync)

```
error[E0522]: `type Orders` has `impl DbRow`, but no matching table exists in the live database
  --> src/main.cto:4:1
   |
   = note: the live schema has drifted from this declaration — update the `type` or the database
```

**Cause:** (Opt-in via `[features] schema-sync = true` in `certo.toml`.) A
`type` with `impl DbRow for X {}` has no matching table in the live
database `DATABASE_URL` points at.

**Fix:** Create the table (e.g. via a migration), or fix the type/table
name mismatch.

---

### E0523  Live column missing (schema-sync)

```
error[E0523]: `orders.discount` has no matching column in the live database
  --> src/main.cto:6:5
```

**Cause:** A field on a `DbRow` type has no matching column in the live
table.

**Fix:** Add the column (via a migration), or remove/rename the field to
match what's really there.

---

### E0524  Live column type mismatch (schema-sync)

```
error[E0524]: `orders.total` type mismatch
  --> src/main.cto:6:12
   |
   = declared as `Int`, live database column is `Decimal`
```

**Cause:** A `DbRow` field's declared Certo type doesn't match the live
column's real database type.

**Fix:** Fix the field's declared type to match the live column, or alter
the live column to match the type.

---

### E0525  Live nullability mismatch (schema-sync)

```
error[E0525]: `orders.total` nullability mismatch
  --> src/main.cto:6:12
   |
   = declared as not nullable, live database column is nullable
```

**Cause:** A `DbRow` field's nullability (`Text` vs `Text?`) doesn't match
whether the live column allows `NULL`.

**Fix:** Add/remove the `?` on the field's type to match the live column,
or alter the live column's nullability.

---

### E0526  Duplicate join alias

```
error[E0526]: alias `"e"` is already used in this query
  --> src/main.cto:14:32
   |
   = note: give each occurrence of a self-joined table a distinct alias via `.fromAs`/`.joinAs`/`.leftJoinAs`
```

**Cause:** `.joinAs`/`.leftJoinAs`/`.fromAs` reuses an alias already in
scope for this query — most commonly a self-join (the same table joined to
itself) that forgot to give each side its own alias.

**Fix:** Give each joined instance of the table a distinct alias, e.g.
`Query.fromAs("Employees", "e") |> Query.joinAs("Employees", "m",
"e.managerId", "m.id")`.

---

## E0600–E0601  HIR lowering errors

### E0600  Unresolved name at lowering

```
error[E0600]: unresolved name `x`
  --> src/main.cto:5:5
```

**Cause:** A name — most commonly an assignment target (`x = 5`) — refers
to a local that was never declared with `val`/`var` in this scope. In
practice this is a belt-and-suspenders check: `resolve`/`typeck` normally
catch this first.

**Fix:** Declare the variable first with `var x = ...`, or check for a
typo.

---

### E0601  Unsupported generic construct

```
error[E0601]: cannot determine the concrete type of this generic
function's return value here — add an explicit type annotation (e.g.
`val x: SomeType = ...`)
  --> src/main.cto:7:13
```

**Cause:** A generic function call's return value stays fully erased
(`Ty::Var`) at the point it's used, with nothing nearby for the compiler to
recover a concrete type from — e.g. `val x = List.empty()` with no
annotation and no way to infer the element type from context.

**Fix:** Add an explicit type annotation: `val x: List<Int> = List.empty()`.

---

## E0700–E0702  Validator rule errors

### E0700  Cycle in rule dependency graph

```
error[E0700]: cycle in rule dependency graph in validator `OrderValidator`:
              checkTotal → checkLines → checkTotal
  --> src/validators.cto:8:5
```

**Cause:** The `after` dependencies between rules in a validator form a cycle.
Rule `a` is `after b` and rule `b` is `after a` (directly or transitively).

**Fix:** Break the cycle by removing or redirecting one `after` dependency.
Rules within a validator must form a DAG (directed acyclic graph).

---

### E0701  `after` references non-existent rule

```
error[E0701]: rule `checkTotal` has `after checkLines` but `checkLines`
              does not exist in this validator
  --> src/validators.cto:12:5
```

**Cause:** An `after` clause names a rule that is not declared in the same
validator.

**Fix:** Check the rule name spelling, or add the missing rule.

---

### E0702  `overrides` references non-existent rule

```
error[E0702]: rule `strictCheck` has `overrides basicCheck` but `basicCheck`
              does not exist in this validator
  --> src/validators.cto:18:5
```

**Cause:** An `overrides` clause names a rule that is not declared in the
same validator.

**Fix:** Check the rule name spelling, or add the missing rule.

---

## E0708–E0709  Temporal / `.age` errors

### E0708  Temporal body is not a Duration

```
error[E0708]: temporal body must be a Duration, found `Int`
  --> src/rules.cto:5:22
   |
   = note: use Duration.days(N), Duration.hours(N), etc.
```

**Cause:** A `temporal` declaration's body expression resolves to a type
other than `Duration`.

**Fix:** Use the `Duration` constructors:

```
// Wrong
temporal gracePeriod = 30

// Fix
temporal gracePeriod = Duration.days(30)
```

---

### E0709  `.age` on non-Timestamp field

```
error[E0709]: `.age` requires a Timestamp field, found `Int`
  --> src/rules.cto:9:18
   |
   = note: only fields of type Timestamp support `.age`
```

**Cause:** The `.age` accessor was used on a field that is not of type
`Timestamp` (or `Timestamp?`).

**Fix:** Ensure the field is declared as `Timestamp` in the `type`, not as
`Int` or `DateTime`.

---

## E0710  Unsupported key/numeric projection type

```
error[E0710]: `List.sortBy`'s key/numeric projection resolved to `Text`,
which isn't supported — only Int/Int8/Int16/Int32/UInt/Float/Float32 are
  --> src/rules.cto:12:29
   |
   = note: only Int/Int8/Int16/Int32/UInt/Float/Float32 are supported —
     Text's ordering isn't lexicographic here and Decimal has no generic
     comparison/addition yet
```

**Cause:** `List.sortBy`/`List.minBy`/`List.maxBy`/`List.sumBy`'s key or
numeric projection function (the second argument) resolved to a type
outside the supported set. This is a deliberate restriction, not a gap
that will be lifted casually: this codebase's raw `<`/`>`/`+` C operators
are only correct for `Int`/`Int8`/`Int16`/`Int32`/`UInt`/`Float`/`Float32`
— `Text`'s `<` is a raw pointer comparison (not lexicographic), and
`Decimal` is a struct with no generic `<`/`+` to synthesize a call to.

**Fix:** Project through a supported numeric type instead — e.g. compare
by a `Text` field's length rather than the `Text` itself, or convert a
`Decimal` to `Float`/`Int` before comparing/summing:

```certo
// Wrong — Text key
val sorted = List.sortBy(products, (p) => p.name)

// Fix — project to a supported type
val sorted = List.sortBy(products, (p) => Text.len(p.name))
```

Note that this restriction applies only to the *projected* key/numeric
type — the list's element type `T` itself is unrestricted, including
struct/record types.

---

## L001–L005  Lint warnings

Lint warnings are emitted by `certo lint`. They do not prevent compilation.
Exit code is `1` if any warnings are found.

**Suppress L001 and L002** by prefixing the name with `_`:

```
fn process(_unusedParam: Int, result: Int): Int = result
val _discarded = sideEffect()
```

---

### L001  Unused parameter

```
warning[L001]: unused parameter `config`
  --> src/users.cto:5:16
   |
 5 | fn createUser(name: Text, config: Config): User = ...
   |                           ^^^^^^^^^^^^^^ never read
```

**Cause:** A function parameter is declared but never read in the function
body.

**Fix:** Either use the parameter, remove it from the signature (and all
callers), or prefix it with `_` to signal intentional non-use:

```
fn createUser(name: Text, _config: Config): User = ...
```

---

### L002  Unused variable

```
warning[L002]: unused variable `temp`
  --> src/orders.cto:12:9
   |
12 |     val temp = computeTotal(lines)
   |         ^^^^ declared but never read
```

**Cause:** A `val` or `var` binding is declared but never used.

**Fix:** Either use the value, remove the binding, or prefix the name with
`_`:

```
val _temp = computeTotal(lines)   // suppresses L002
```

---

### L003  Assigned but never read

```
warning[L003]: assigned but never read
  --> src/orders.cto:8:5
   |
 8 |     count = count + 1
   |     ^^^^^ this value is never used before it is overwritten
```

**Cause:** A `var` is written to but the written value is never read before
the variable is assigned again or the scope ends. The write is dead code.

**Fix:** Remove the dead assignment, or check whether the logic is correct
(perhaps you meant to use the value before overwriting it).

---

### L004  Unreachable statement

```
warning[L004]: unreachable statement
  --> src/main.cto:15:5
   |
15 |     println("done")
   |     ^^^^^^^^^^^^^^^ after panic/todo/unreachable, this is never reached
```

**Cause:** A statement follows a call to `panic`, `todo`, or `unreachable`,
or follows a `guard ... else return` that always exits.

**Fix:** Remove the unreachable code. If you intended the code to be reached,
move it before the terminating expression.

---

### L005  Guard condition is a literal bool

```
warning[L005]: guard condition is always `true`
  --> src/utils.cto:7:11
   |
 7 |     guard true else return Err("never")
   |           ^^^^ this guard never exits
```

**Cause:** The condition in a `guard` clause is a literal `true` or `false`,
making the guard either always-exit (`false`) or never-exit (`true`).

**Fix:** Replace the literal with a real condition, or remove the guard
entirely if it is not needed.

---

## Parse errors

Parse errors have no `E` code — they are reported as plain error messages
with a source location. Common parse errors:

| Message | Typical cause |
|---|---|
| `unexpected token '}'` | Mismatched brace; check all `{` are closed |
| `expected ':'` | Missing `:` in a field type annotation |
| `expected expression` | Dangling operator or empty block |
| `expected identifier` | Keyword used where a name is expected |
| `unterminated string literal` | Missing closing `"` |
| `unexpected end of file` | Unclosed `{`, `(`, or string |
| `integer literal out of range` | Value outside ±2⁶³ |

Parse errors are reported with the line and column of the first unexpected
token. If the location looks wrong, check the **line above** — the real
mistake is often one line up from where the parser noticed the problem.

---

## Reading error output

```
error[E0200]: type mismatch: expected `Text`, found `Int`
  --> src/main.cto:8:20
   |
 8 |     println(userId)
   |             ^^^^^^ this has type `Int`
   |
   = note: use `intToText(n)` to convert an Int to Text
```

| Part | Meaning |
|---|---|
| `error[E0200]` | Error code — look it up in this document |
| `src/main.cto:8:20` | File, line, column |
| The `^^^` underline | Span of the offending expression |
| `= note:` | Context-specific fix hint |

Errors that span multiple lines have the form `8:1–12:3` (start line:col–end
line:col).

### Colour output

Colour is enabled when stderr is a terminal and `NO_COLOR` is not set. To
disable permanently:

```sh
export NO_COLOR=1
```

To disable for a single invocation:

```sh
NO_COLOR=1 certo build src/main.cto
```
