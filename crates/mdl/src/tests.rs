use crate::*;
use certo_sdl::{compile, SchemaIR};

fn ir(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    ir.unwrap_or_else(|| panic!("schema failed: {d:?}"))
}

fn ops(before: &str, after: &str) -> Vec<String> {
    diff(&ir(before), &ir(after)).ops.iter().map(Op::describe).collect()
}

const USERS: &str = "table users { id: uuid primary key  email: text not null }";

#[test]
fn identical_schemas_produce_empty_plan() {
    let s = ir("enum E { a } type T { x: int } table t { id: int primary key e: E } index i on t (id)");
    assert!(diff(&s, &s).is_empty());
}

#[test]
fn initial_migration_from_empty() {
    let plan = diff(
        &SchemaIR::empty(),
        &ir("enum Role { admin }
             table users { id: uuid primary key role: Role  posts: posts -> many }
             table posts { id: uuid primary key author: users -> one }
             index users_role on users (role)
             constraint c on users using role != admin"),
    );
    let d: Vec<_> = plan.ops.iter().map(Op::describe).collect();
    assert_eq!(d, [
        "+ enum Role",
        "+ table posts",
        "+ table users",
        "+ relationship posts.author",
        "+ relationship users.posts",
        "+ index users_role on users",
        "+ constraint c on users",
    ]);
    // CreateTable carries columns only
    let Op::CreateTable { definition } = &plan.ops[1] else { panic!() };
    assert!(definition.relationships.is_empty() && definition.indexes.is_empty());
    assert!(!definition.columns.is_empty());
}

#[test]
fn column_add_alter_drop() {
    let d = ops(USERS, "table users { id: uuid primary key  email: text  age: int }");
    assert_eq!(d, ["+ column users.age", "~ column users.email"]);

    let d = ops(USERS, "table users { id: uuid primary key }");
    assert_eq!(d, ["- column users.email"]);
}

#[test]
fn destructive_flags() {
    let drop_col = diff(&ir(USERS), &ir("table users { id: uuid primary key }"));
    assert!(drop_col.has_destructive());

    // widening nullability is safe
    let relax = diff(&ir(USERS), &ir("table users { id: uuid primary key email: text }"));
    assert!(!relax.has_destructive());
    // tightening is not
    let tighten = diff(&ir("table t { id: int primary key x: text }"), &ir("table t { id: int primary key x: text not null }"));
    assert!(tighten.has_destructive());
    // changing type is not
    let retype = diff(&ir("table t { id: int primary key x: int }"), &ir("table t { id: int primary key x: text }"));
    assert!(retype.has_destructive());

    assert!(!diff(&SchemaIR::empty(), &ir(USERS)).has_destructive());
    assert!(diff(&ir(USERS), &SchemaIR::empty()).has_destructive());
}

#[test]
fn enum_changes() {
    let d = ops("enum E { a, b }", "enum E { a, c }");
    assert_eq!(d, ["+ enum variant E.c", "- enum variant E.b"]);
    assert_eq!(ops("enum E { a }", ""), ["- enum E"]);
}

#[test]
fn ordering_puts_creates_before_uses_and_drops_last() {
    // new enum used by a new column on an existing table; old column dropped
    let d = ops(
        "table t { id: int primary key old: text }",
        "enum E { a } table t { id: int primary key e: E }",
    );
    assert_eq!(d, ["+ enum E", "+ column t.e", "- column t.old"]);

    // column switched to a new enum, old enum dropped: alter happens before drop
    let d = ops(
        "enum Old { a } table t { id: int primary key e: Old }",
        "enum New { a } table t { id: int primary key e: New }",
    );
    assert_eq!(d, ["+ enum New", "~ column t.e", "- enum Old"]);
}

#[test]
fn composite_types_created_dependencies_first() {
    let d = ops("", "type Outer { i: Inner } type Inner { x: int }");
    assert_eq!(d, ["+ type Inner", "+ type Outer"]);
    // and dropped dependents first
    let d = ops("type Outer { i: Inner } type Inner { x: int }", "");
    assert_eq!(d, ["- type Outer", "- type Inner"]);
}

#[test]
fn changed_index_and_constraint_are_dropped_then_recreated() {
    let base = "table t { id: int primary key a: int b: int } ";
    let d = ops(
        &format!("{base} index i on t (a) constraint c on t using a > 1"),
        &format!("{base} index i on t (a, b) constraint c on t using a > 2"),
    );
    assert_eq!(d, [
        "- index i on t",
        "- constraint c on t",
        "+ index i on t",
        "+ constraint c on t",
    ]);
}

#[test]
fn relationship_changes() {
    let base = "table u { id: int primary key } ";
    let d = ops(
        &format!("{base} table t {{ id: int primary key r: u -> one }}"),
        &format!("{base} table t {{ id: int primary key r: u -> many }}"),
    );
    assert_eq!(d, ["- relationship t.r", "+ relationship t.r"]);
}

#[test]
fn dropping_a_table_drops_relationships_pointing_at_it_first() {
    let d = ops(
        "table u { id: int primary key } table t { id: int primary key r: u -> one }",
        "table t { id: int primary key }",
    );
    assert_eq!(d, ["- relationship t.r", "- table u"]);
}

#[test]
fn plan_json_round_trips() {
    let plan = diff(&SchemaIR::empty(), &ir(USERS));
    assert_eq!(MigrationPlan::from_json(&plan.to_json()).unwrap(), plan);
    let v: serde_json::Value = serde_json::from_str(&plan.to_json()).unwrap();
    assert_eq!(v["ops"][0]["op"], "create_table");
}

#[test]
fn diff_is_deterministic_across_declaration_order() {
    let a = diff(&SchemaIR::empty(), &ir("table b { id: int primary key } table a { id: int primary key }"));
    let b = diff(&SchemaIR::empty(), &ir("table a { id: int primary key } table b { id: int primary key }"));
    assert_eq!(a, b);
}

const FK_OLD: &str = "table users { id: uuid primary key }
                      table posts { id: uuid primary key  author_id: uuid references users }";

#[test]
fn foreign_keys_are_separate_ops() {
    // initial migration: tables first, keys after
    let plan = diff(&SchemaIR::empty(), &ir(FK_OLD));
    let d: Vec<_> = plan.ops.iter().map(Op::describe).collect();
    assert_eq!(d, ["+ table posts", "+ table users", "+ foreign key posts.author_id -> users.id"]);
    let Op::CreateTable { definition } = &plan.ops[0] else { panic!() };
    assert!(definition.columns.iter().all(|c| c.references.is_none()));

    // adding a key to an existing column changes nothing else
    let d = ops(
        "table users { id: uuid primary key } table posts { id: uuid primary key author_id: uuid }",
        FK_OLD,
    );
    assert_eq!(d, ["+ foreign key posts.author_id -> users.id"]);
    assert!(!diff(&ir(FK_OLD), &ir(FK_OLD)).has_destructive());
}

#[test]
fn foreign_key_change_and_removal() {
    let d = ops(
        "table users { id: uuid primary key  email: uuid unique } table posts { id: uuid primary key a: uuid references users }",
        "table users { id: uuid primary key  email: uuid unique } table posts { id: uuid primary key a: uuid references users(email) }",
    );
    assert_eq!(d, ["- foreign key posts.a", "+ foreign key posts.a -> users.email"]);
    let d = ops(FK_OLD, "table users { id: uuid primary key } table posts { id: uuid primary key author_id: uuid }");
    assert_eq!(d, ["- foreign key posts.author_id"]);
}

#[test]
fn mutually_referencing_tables_can_be_created() {
    let d = ops(
        "",
        "table a { id: int primary key  b_id: int references b }
         table b { id: int primary key  a_id: int references a }",
    );
    assert_eq!(d, [
        "+ table a", "+ table b",
        "+ foreign key a.b_id -> b.id", "+ foreign key b.a_id -> a.id",
    ]);
}

#[test]
fn dropped_tables_are_ordered_by_foreign_keys() {
    // posts references users, so posts must go first even though it sorts later
    let d = ops(FK_OLD, "");
    assert_eq!(d, ["- table posts", "- table users"]);
    // dropping a referenced table while the referrer survives drops the key first
    let d = ops(FK_OLD, "table posts { id: uuid primary key author_id: uuid }");
    assert_eq!(d, ["- foreign key posts.author_id", "- table users"]);
}

#[test]
fn referential_action_change_replaces_the_key() {
    let a = "table u { id: int primary key } table t { id: int primary key r: int references u }";
    let b = "table u { id: int primary key } table t { id: int primary key r: int references u on delete cascade }";
    assert_eq!(ops(a, b), ["- foreign key t.r", "+ foreign key t.r -> u.id on delete cascade"]);
}

#[test]
fn widening_a_type_is_safe_narrowing_is_not() {
    let safe = [
        ("s: varchar(20)", "s: varchar(40)"),
        ("s: varchar(20)", "s: text"),
        ("s: char(3)", "s: varchar(10)"),
        ("n: smallint", "n: int"),
        ("n: int", "n: bigint"),
        ("n: bigint", "n: decimal"),
        ("n: real", "n: float"),
        ("n: decimal(10,2)", "n: decimal(12,2)"),
        ("n: decimal(10,2)", "n: decimal(14,4)"),
        ("n: decimal(10,2)", "n: decimal"),
    ];
    for (a, b) in safe {
        let d = diff(&ir(&format!("table t {{ id: int primary key  {a} }}")), &ir(&format!("table t {{ id: int primary key  {b} }}")));
        assert!(!d.is_empty() && !d.has_destructive(), "{a} -> {b} should be a safe widening");
    }
    let lossy = [
        ("s: varchar(40)", "s: varchar(20)"),
        ("s: text", "s: varchar(255)"),
        ("n: bigint", "n: int"),
        ("n: int", "n: smallint"),
        ("n: decimal(12,2)", "n: decimal(10,2)"),
        ("n: decimal(10,2)", "n: decimal(12,1)"), // less scale: rounds
        ("n: decimal(10,2)", "n: decimal(11,4)"), // fewer integer digits (8 -> 7)
        ("n: float", "n: real"),
        ("n: decimal", "n: decimal(10,2)"),
        ("n: int", "n: text"),
    ];
    for (a, b) in lossy {
        let d = diff(&ir(&format!("table t {{ id: int primary key  {a} }}")), &ir(&format!("table t {{ id: int primary key  {b} }}")));
        assert!(d.has_destructive(), "{a} -> {b} can lose data");
    }
}


// ---- serial, identity, sequences -------------------------------------------- //

#[test]
fn sequences_are_created_before_tables_and_dropped_last() {
    let d = ops(
        "",
        "sequence s start 100  table t { id: bigint primary key default nextval(s) }",
    );
    assert_eq!(d, ["+ sequence s", "+ table t"]);
    let d = ops("sequence s start 100  table t { id: bigint primary key default nextval(s) }", "table t { id: bigint primary key }");
    assert_eq!(d, ["~ column t.id", "- sequence s"], "the default stops using it before it is dropped");
}

#[test]
fn sequence_changes_and_destructiveness() {
    let d = ops("sequence s", "sequence s increment 5 cache 10 cycle");
    assert_eq!(d, ["~ sequence s"]);
    let plan = diff(&ir("sequence s"), &ir("sequence s increment 5"));
    assert!(!plan.has_destructive());
    assert!(diff(&ir("sequence s"), &SchemaIR::empty()).has_destructive(), "dropping loses the counter");
    assert!(!diff(&SchemaIR::empty(), &ir("sequence s")).has_destructive());
}

#[test]
fn serial_and_identity_columns_travel_with_the_column() {
    let plan = diff(&SchemaIR::empty(), &ir("table t { id: serial primary key  n: int generated always }"));
    let Op::CreateTable { definition } = &plan.ops[0] else { panic!() };
    assert_eq!(definition.columns[0].generated, Some(certo_sdl::Generation::Serial));
    assert_eq!(definition.columns[1].generated, Some(certo_sdl::Generation::Always));
    assert!(!plan.has_destructive());
    // adding one to an existing table
    assert_eq!(ops("table t { id: int primary key }", "table t { id: int primary key  n: serial }"), ["+ column t.n"]);
}

#[test]
fn changing_a_columns_generation() {
    use certo_sdl::Generation::*;
    // start from NOT NULL columns so only the generation changes (identity/serial imply NOT NULL)
    let col = |g: &str| format!("table t {{ id: int primary key  n: int not null{g} }}");
    let cases: &[(&str, &str, bool)] = &[
        ("", " generated always", false),                        // adding: safe
        ("", " generated by default", false),
        (" generated always", " generated by default", false),   // flipping identity: safe
        (" generated by default", " generated always", false),
        (" generated always", "", true),                          // removing: loses the counter
        (" generated by default", "", true),
        // serial spelled through the type, so use its own table text
    ];
    for (a, b, destructive) in cases {
        let plan = diff(&ir(&col(a)), &ir(&col(b)));
        assert_eq!(plan.ops.iter().map(Op::describe).collect::<Vec<_>>(), ["~ column t.n"], "{a:?} -> {b:?}");
        assert_eq!(plan.has_destructive(), *destructive, "{a:?} -> {b:?}");
    }
    // serial: none <-> serial and serial <-> identity
    let none = ir("table t { id: int primary key  n: int not null }");
    let serial = ir("table t { id: int primary key  n: serial }");
    let ident = ir("table t { id: int primary key  n: int generated always }");
    assert!(!diff(&none, &serial).has_destructive());
    assert!(diff(&serial, &none).has_destructive());
    assert!(diff(&serial, &ident).has_destructive());
    assert!(diff(&ident, &serial).has_destructive());
    assert!(generation_lossy(Some(Serial), None) && !generation_lossy(None, Some(Serial)));
    // and adding an identity to a NULLABLE column is flagged, because it also becomes NOT NULL
    let nullable = ir("table t { id: int primary key  n: int }");
    assert!(diff(&nullable, &ident).has_destructive());
}

#[test]
fn a_sequence_rides_the_plan_json() {
    let plan = diff(&SchemaIR::empty(), &ir("sequence s start 5  table t { id: bigint primary key default nextval(s) }"));
    assert_eq!(MigrationPlan::from_json(&plan.to_json()).unwrap(), plan);
    assert!(plan.to_json().contains("\"op\": \"create_sequence\""));
}

// ---- views -------------------------------------------------------------- //

const WITH_VIEW: &str = "table users { id: uuid primary key  email: text not null  age: int }
                         view adults on users (id, email) where age >= 18";

#[test]
fn views_are_created_last_and_dropped_first() {
    // new view on an existing table; and a new table with a view on it
    assert_eq!(ops(USERS, WITH_VIEW), ["+ column users.age", "+ view adults"]);
    assert_eq!(
        ops("", WITH_VIEW),
        ["+ table users", "+ view adults"]
    );
    // removed
    assert_eq!(ops(WITH_VIEW, "table users { id: uuid primary key  email: text not null  age: int }"), ["- view adults"]);
    // changed: dropped before, created after
    assert_eq!(
        ops(WITH_VIEW, &WITH_VIEW.replace("age >= 18", "age >= 21")),
        ["- view adults", "+ view adults"]
    );
    assert!(diff(&ir(WITH_VIEW), &ir(WITH_VIEW)).is_empty());
}

#[test]
fn a_view_over_a_table_being_altered_is_recreated() {
    let altered = WITH_VIEW.replace("email: text not null", "email: varchar(320) not null");
    let got = ops(WITH_VIEW, &altered);
    assert_eq!(got, ["- view adults", "~ column users.email", "+ view adults"]);
    // a column dropped along with the view that used it
    let gone = "table users { id: uuid primary key  email: text not null }";
    assert_eq!(ops(WITH_VIEW, gone), ["- view adults", "- column users.age"]);
    // unrelated table changes leave it alone
    let other = format!("{WITH_VIEW}\ntable t {{ id: int primary key }}");
    assert_eq!(ops(WITH_VIEW, &other), ["+ table t"]);
}
