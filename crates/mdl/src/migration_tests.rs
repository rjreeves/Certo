use crate::*;
use certo_sdl::{compile, SchemaIR};

fn ir(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    ir.unwrap_or_else(|| panic!("schema failed: {d:?}"))
}

fn plan(before: &str, after: &str, mdl: &str) -> Vec<String> {
    let (plan, d) = compile_migration(&ir(before), &ir(after), mdl);
    let plan = plan.unwrap_or_else(|| panic!("migration failed: {d:?}"));
    plan.ops.iter().map(Op::describe).collect()
}

fn codes(before: &str, after: &str, mdl: &str) -> Vec<String> {
    let (plan, d) = compile_migration(&ir(before), &ir(after), mdl);
    assert!(plan.is_none(), "expected failure");
    d.into_iter().map(|x| x.code).collect()
}

// ---- renames ------------------------------------------------------- //

#[test]
fn without_mdl_a_rename_is_drop_and_add() {
    let d = diff(&ir("table a { id: int primary key }"), &ir("table b { id: int primary key }"));
    assert!(d.has_destructive());
}

#[test]
fn rename_table_is_one_op_and_not_destructive() {
    let (p, _) = compile_migration(
        &ir("table a { id: int primary key  x: text }"),
        &ir("table b { id: int primary key  x: text }"),
        "rename table a -> b",
    );
    let p = p.unwrap();
    assert_eq!(p.ops.iter().map(Op::describe).collect::<Vec<_>>(), ["~ rename table a -> b"]);
    assert!(!p.has_destructive());
}

#[test]
fn rename_table_updates_references_to_it() {
    // posts.author_id references users; users is renamed to accounts
    let d = plan(
        "table users { id: uuid primary key } table posts { id: uuid primary key author_id: uuid references users  a: users -> one }",
        "table accounts { id: uuid primary key } table posts { id: uuid primary key author_id: uuid references accounts  a: accounts -> one }",
        "rename table users -> accounts",
    );
    assert_eq!(d, ["~ rename table users -> accounts"]);
}

#[test]
fn rename_column_carries_indexes_constraints_and_keys() {
    let before = "table u { id: int primary key  n: int } table t { id: int primary key  r: int references u(id) }
                  index i on u (n) constraint c on u using n > 0";
    let after = "table u { id: int primary key  m: int } table t { id: int primary key  r: int references u(id) }
                 index i on u (m) constraint c on u using m > 0";
    assert_eq!(plan(before, after, "rename column u.n -> m"), ["~ rename column u.n -> m"]);

    // a foreign key that targets the renamed column follows it
    let before = "table u { id: int primary key  k: int unique } table t { id: int primary key  r: int references u(k) }";
    let after = "table u { id: int primary key  key2: int unique } table t { id: int primary key  r: int references u(key2) }";
    assert_eq!(plan(before, after, "rename column u.k -> key2"), ["~ rename column u.k -> key2"]);
}

#[test]
fn rename_combined_with_a_real_change() {
    let d = plan(
        "table a { id: int primary key  x: text }",
        "table b { id: int primary key  y: text not null  z: int }",
        "rename table a -> b\nrename column a.x -> y",
    );
    assert_eq!(d, [
        "~ rename table a -> b",
        "~ rename column b.x -> y",
        "~ column b.y",
        "+ column b.z",
    ]);
}

#[test]
fn rename_errors() {
    let a = "table a { id: int primary key }";
    let b = "table b { id: int primary key }";
    assert_eq!(codes(a, b, "rename table ghost -> b"), ["MDL301"]);
    assert_eq!(codes(a, b, "rename table a -> ghost"), ["MDL302"]);
    // old name must actually disappear, new name must be new
    assert_eq!(codes(a, &format!("{a} {b}"), "rename table a -> b"), ["MDL303"]);
    assert_eq!(codes(&format!("{a} {b}"), b, "rename table a -> b"), ["MDL303"]);
    assert_eq!(codes(a, b, "rename table a -> b\nrename table a -> b"), ["MDL304"]);

    let c1 = "table t { id: int primary key  x: int }";
    let c2 = "table t { id: int primary key  y: int }";
    assert_eq!(codes(c1, c2, "rename column ghost.x -> y"), ["MDL301"]);
    assert_eq!(codes(c1, c2, "rename column t.ghost -> y"), ["MDL301"]);
    assert_eq!(codes(c1, c2, "rename column t.x -> ghost"), ["MDL302"]);
    assert_eq!(codes(c1, "table t { id: int primary key  x: int  y: int }", "rename column t.x -> y"), ["MDL303"]);
    assert_eq!(codes("table t { id: int primary key  x: int  y: int }", "table t { id: int primary key  y: int  z: int }", "rename column t.x -> z\nrename column t.x -> z"), ["MDL304"]);
    // table gone from the new schema
    assert_eq!(codes(c1, b, "rename column t.x -> y"), ["MDL302"]);
}

// ---- remap --------------------------------------------------------- //

const ENUM_OLD: &str = "enum Role { admin, user, guest } table users { id: int primary key  role: Role default guest }";
const ENUM_NEW: &str = "enum Role { admin, user } table users { id: int primary key  role: Role default user }";

#[test]
fn remap_replaces_the_unsupported_variant_removal() {
    // without MDL the removal is planned but cannot be lowered
    assert!(diff(&ir(ENUM_OLD), &ir(ENUM_NEW)).ops.iter().any(|o| matches!(o, Op::RemoveEnumVariant { .. })));

    let (p, d) = compile_migration(&ir(ENUM_OLD), &ir(ENUM_NEW), "remap Role.guest -> user");
    assert!(d.is_empty(), "{d:?}");
    let p = p.unwrap();
    assert!(!p.ops.iter().any(|o| matches!(o, Op::RemoveEnumVariant { .. })));
    let Some(Op::RecreateEnum { name, mappings, columns, after, .. }) =
        p.ops.iter().find(|o| matches!(o, Op::RecreateEnum { .. }))
    else { panic!("{:?}", p.ops) };
    assert_eq!(name, "Role");
    assert_eq!((mappings[0].from.as_str(), mappings[0].to.as_str()), ("guest", "user"));
    assert_eq!(after.variants, ["admin", "user"]);
    assert_eq!(columns.len(), 1);
    assert_eq!(columns[0].table, "users");
    assert!(p.has_destructive());
}

#[test]
fn remap_runs_after_column_changes() {
    let d = plan(ENUM_OLD, ENUM_NEW, "remap Role.guest -> user");
    assert_eq!(d, ["~ column users.role", "~ recreate enum Role (remap guest -> user)"]);
}

#[test]
fn several_removed_variants_become_one_recreate() {
    let d = plan(
        "enum R { a, b, c, d } table t { id: int primary key r: R }",
        "enum R { a, d } table t { id: int primary key r: R }",
        "remap R.b -> a\nremap R.c -> d",
    );
    assert_eq!(d, ["~ recreate enum R (remap b -> a, c -> d)"]);
}

#[test]
fn remap_errors() {
    assert_eq!(codes(ENUM_OLD, ENUM_NEW, "remap Ghost.guest -> user"), ["MDL301"]);
    assert_eq!(codes(ENUM_OLD, ENUM_NEW, "remap Role.ghost -> user"), ["MDL301"]);
    assert_eq!(codes(ENUM_OLD, ENUM_NEW, "remap Role.user -> admin"), ["MDL303"]); // not removed
    assert_eq!(codes(ENUM_OLD, ENUM_NEW, "remap Role.guest -> ghost"), ["MDL302"]);
    assert_eq!(codes(ENUM_OLD, ENUM_NEW, "remap Role.guest -> user\nremap Role.guest -> admin"), ["MDL304"]);
    // every removed variant needs a mapping once any is given
    assert_eq!(
        codes(
            "enum R { a, b, c } table t { id: int primary key r: R }",
            "enum R { a } table t { id: int primary key r: R }",
            "remap R.b -> a"
        ),
        ["MDL306"]
    );
    // enums used inside composite types are out of scope
    assert_eq!(
        codes(
            "enum R { a, b } type T { r: R } table t { id: int primary key }",
            "enum R { a } type T { r: R } table t { id: int primary key }",
            "remap R.b -> a"
        ),
        ["MDL305"]
    );
}

// ---- backfill ------------------------------------------------------ //

#[test]
fn backfill_a_new_not_null_column() {
    let (old, new) = (
        "table t { id: int primary key  name: text }",
        "table t { id: int primary key  name: text  slug: text not null }",
    );
    let d = plan(old, new, "backfill t.slug = lower(name)");
    assert_eq!(d, ["+ column t.slug", "~ backfill t.slug (then NOT NULL)"]);
    let (p, _) = compile_migration(&ir(old), &ir(new), "backfill t.slug = lower(name)");
    let p = p.unwrap();
    let Op::AddColumn { column, .. } = &p.ops[0] else { panic!() };
    assert!(column.nullable, "added nullable first; the backfill tightens it");
    assert!(!p.has_destructive(), "a backfilled NOT NULL column is safe");
    // ...whereas the same change without a backfill is flagged when tightening existing data
    let plain = diff(&ir("table t { id: int primary key  x: int }"), &ir("table t { id: int primary key  x: int not null }"));
    assert!(plain.has_destructive());
}

#[test]
fn backfill_when_tightening_an_existing_column() {
    let (old, new) = ("table t { id: int primary key  x: int }", "table t { id: int primary key  x: int not null }");
    let (p, _) = compile_migration(&ir(old), &ir(new), "backfill t.x = 0");
    let p = p.unwrap();
    assert_eq!(
        p.ops.iter().map(Op::describe).collect::<Vec<_>>(),
        ["~ column t.x", "~ backfill t.x (then NOT NULL)"]
    );
    let Op::AlterColumn { after, .. } = &p.ops[0] else { panic!() };
    assert!(after.nullable, "stays nullable until the backfill has run");
    assert!(!p.has_destructive());
}

#[test]
fn backfill_lands_after_all_column_changes_and_before_constraints() {
    let d = plan(
        "table t { id: int primary key  a: int }",
        "table t { id: int primary key  a: int  s: text not null  z: int } constraint c on t using a > 0",
        "backfill t.s = \"x\"",
    );
    assert_eq!(d, [
        "+ column t.s", "+ column t.z",
        "~ backfill t.s (then NOT NULL)",
        "+ constraint c on t",
    ]);
}

#[test]
fn backfill_errors() {
    let old = "table t { id: int primary key }";
    let new = "table t { id: int primary key  s: text not null }";
    assert_eq!(codes(old, new, "backfill ghost.s = \"x\""), ["MDL302"]);
    assert_eq!(codes(old, new, "backfill t.ghost = \"x\""), ["MDL311"]);
    assert_eq!(codes(old, new, "backfill t.s = 1"), ["MDL312"]);
    assert_eq!(codes(old, new, "backfill t.s = nope"), ["SDL210"]);
    assert_eq!(codes(old, new, "backfill t.s = \"x\"\nbackfill t.s = \"y\""), ["MDL304"]);
    // nothing to backfill: column is nullable / already NOT NULL / unchanged
    assert_eq!(codes(old, "table t { id: int primary key  s: text }", "backfill t.s = \"x\""), ["MDL320"]);
    assert_eq!(codes(new, new, "backfill t.s = \"x\""), ["MDL320"]);
}

// ---- blocks -------------------------------------------------------- //

#[test]
fn blocks_bracket_the_generated_plan() {
    let d = plan(
        "table t { id: int primary key  a: int }",
        "table t { id: int primary key  a: int  b: int }",
        "after { update t set b = a }
         before { update t set a = 0 where is_null(a) }
         after { sql postgres \"ANALYZE t\" }",
    );
    assert_eq!(d, ["~ update data in t", "+ column t.b", "~ update data in t", "~ raw sql (postgres)"]);
}

#[test]
fn before_sees_the_old_schema_and_after_the_new() {
    let old = "table t { id: int primary key  a: int }";
    let new = "table t { id: int primary key  b: int }";
    // `a` exists only in the old schema, `b` only in the new one
    assert!(compile_migration(&ir(old), &ir(new), "before { update t set a = 1 }").0.is_some());
    assert!(compile_migration(&ir(old), &ir(new), "after { update t set b = 1 }").0.is_some());
    assert_eq!(codes(old, new, "after { update t set a = 1 }"), ["MDL311"]);
    assert_eq!(codes(old, new, "before { update t set b = 1 }"), ["MDL311"]);
}

#[test]
fn update_step_errors() {
    let s = "enum E { x, y } table t { id: int primary key  n: int  name: text  e: E }";
    assert_eq!(codes(s, s, "after { update ghost set n = 1 }"), ["MDL310"]);
    assert_eq!(codes(s, s, "after { update t set ghost = 1 }"), ["MDL311"]);
    assert_eq!(codes(s, s, "after { update t set n = \"a\" }"), ["MDL312"]);
    assert_eq!(codes(s, s, "after { update t set n = 1, n = 2 }"), ["MDL314"]);
    assert_eq!(codes(s, s, "after { update t set n = 1 where n + 1 }"), ["MDL313"]);
    assert_eq!(codes(s, s, "after { update t set n = 1 where nope > 1 }"), ["SDL210"]);
    // enum variants resolve from the assigned column's type
    assert!(compile_migration(&ir(s), &ir(s), "after { update t set e = y where e == x }").0.is_some());
}

#[test]
fn sql_step_errors() {
    let s = "table t { id: int primary key }";
    assert_eq!(codes(s, s, "after { sql oracle \"x\" }"), ["MDL315"]);
    assert_eq!(codes(s, s, "after { sql \"   \" }"), ["MDL316"]);
}

#[test]
fn syntax_errors_stop_before_planning() {
    let s = "table t { id: int primary key }";
    let (p, d) = compile_migration(&ir(s), &ir(s), "rename bogus");
    assert!(p.is_none());
    assert!(d.iter().all(|x| x.code.starts_with("SDL1")), "{d:?}");
}

#[test]
fn empty_mdl_is_the_plain_diff() {
    let old = "table t { id: int primary key }";
    let new = "table t { id: int primary key  x: int }";
    assert_eq!(plan(old, new, ""), ["+ column t.x"]);
    assert_eq!(plan(old, new, "// just a comment\n"), ["+ column t.x"]);
}

#[test]
fn plan_with_mdl_ops_round_trips_through_json() {
    let (p, _) = compile_migration(
        &ir(ENUM_OLD),
        &ir(ENUM_NEW),
        "remap Role.guest -> user\nafter { update users set id = id }\nafter { sql \"SELECT 1\" }",
    );
    let p = p.unwrap();
    assert_eq!(MigrationPlan::from_json(&p.to_json()).unwrap(), p);
    assert!(p.to_json().contains("\"op\": \"recreate_enum\""));
}


#[test]
fn renaming_a_column_updates_every_expression_form_that_uses_it() {
    // regression: `not`, `is null` and `in` must be walked when a column is renamed
    let before = "table t { id: int primary key  n: int }
                  constraint a on t using not n == 1
                  constraint b on t using n is null or n in (1, 2)
                  constraint c on t using n is not null and not (n in (3))";
    let after = "table t { id: int primary key  m: int }
                 constraint a on t using not m == 1
                 constraint b on t using m is null or m in (1, 2)
                 constraint c on t using m is not null and not (m in (3))";
    assert_eq!(plan(before, after, "rename column t.n -> m"), ["~ rename column t.n -> m"]);
}
