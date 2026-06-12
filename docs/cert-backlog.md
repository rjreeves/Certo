postgres://postgres:postgres1@127.0.0.1:5432/fireworks

 Transaction safety / RAII-style scoping

 NULL handling is lossy

Option C (savepoints) — pure C change, fixes a real correctness bug in withTransaction right now, no language work.
Option A (withConnection) — closes the leak gap, takes ~30 min, follows the established pattern.
Option B (defer) — the right long-term answer, but touches MIR + codegen and deserves its own focused session.
 
dbQueryTyped — the generic T isn't enforced at the call site today

The Certo side declares the return as List<T> inferred from the mapper, but there's no structural check that the mapper matches the schema type. A validator or trait-bound approach could enforce this:

fn dbQueryTyped<T: DbRow>(conn: Int, sql: Text, params: List<Text>, mapper: fn(List<Text>): T): List<T> [io]
Requiring a DbRow trait impl (auto-derived from certo db pull output) would give compile-time column-count and type checking.



 Streaming / cursor-based queries


 Migration schema validation

crates/dbschema builds a Schema from type declarations and check_migrations.rs exists — but the validation is currently only checking that migration ops reference known types. There's no diff-based "does the current migration set match the live DB?" check beyond certo db status. A certo db diff command that compares pg_catalog to the migration-derived schema would catch drift


