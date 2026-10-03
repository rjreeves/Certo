# Database platform docs

The database platform (SDL, MDL, QL, SQL adapters, runner, C ABI) lives in the `crates/` named below.

| Document | What it is |
|----------|------------|
| [ebnf.md](ebnf.md) | The grammar and rules of the three languages: SDL (schemas), MDL (migrations) and QL (queries, mutations, subqueries, set operations, `with`, window functions), with their error codes |
| [Plan.md](Plan.md) | The original architecture plan: SchemaIR as the foundation, and what is built on it (this is where "MeDL (metrics)" is mentioned; it is postponed and has no syntax yet) |

| Crate | README |
|-------|--------|
| `crates/sdl`, `crates/mdl` | schema language and migrations (grammar in `ebnf.md`) |
| `crates/sql` | [dialects and lowering](../../crates/sql/README.md) (PostgreSQL, SQLite) |
| `crates/runner` | [migration runner](../../crates/runner/README.md) |
| `crates/ql` | [queries and code generation](../../crates/ql/README.md) |
| `crates/capi` | [C ABI](../../crates/capi/README.md) and the `Certo.Native` NuGet package (`packaging/dotnet`) |
