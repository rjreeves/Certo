use crate::*;

const GOOD: &str = "
enum Role { admin, user }
type Address { street: text city: text }
table users {
    id: uuid primary key default gen_uuid()
    email: text not null unique
    role: Role default user
    age: int default 18
    home: Address
    created: timestamp default now()
    posts: posts -> many
}
table posts {
    id: uuid primary key
    author: users -> one
    title: text not null
}
index users_email on users (email)
constraint adult on users using age >= 18 and role != admin
";

fn compile_ok(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    let errs: Vec<_> = d.iter().filter(|x| x.severity == certo_diagnostics::Severity::Error).collect();
    assert!(errs.is_empty(), "unexpected errors: {errs:?}");
    ir.expect("IR")
}

fn errors(src: &str) -> Vec<String> {
    let (ir, d) = compile(src);
    assert!(ir.is_none(), "expected failure");
    d.iter().filter(|x| x.severity == certo_diagnostics::Severity::Error).map(|x| x.code.clone()).collect()
}

#[test]
fn builds_ir() {
    let ir = compile_ok(GOOD);
    // sorted by name
    let names: Vec<_> = ir.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["posts", "users"]);
    let users = ir.table("users").unwrap();
    let id = users.column("id").unwrap();
    assert!(id.primary_key && !id.nullable);
    assert!(matches!(id.default, Some(ExprIR::Call { .. })));
    assert!(users.column("email").unwrap().unique);
    assert!(users.column("role").unwrap().nullable);
    assert_eq!(users.column("home").unwrap().ty, TypeIR::Composite("Address".into()));
    assert!(matches!(
        users.column("role").unwrap().default,
        Some(ExprIR::EnumVariant { ref variant, .. }) if variant == "user"
    ));
    assert_eq!(users.relationships[0].target, "posts");
    assert_eq!(users.indexes[0].columns, ["email"]);
    assert_eq!(users.constraints.len(), 1);
}

#[test]
fn json_is_deterministic_and_round_trips() {
    let a = compile_ok(GOOD);
    // same schema, declarations reordered and re-spaced
    let b = compile_ok(
        "table posts { id: uuid primary key  author: users -> one  title: text not null }\n\
         index users_email on users (email)\n\
         constraint adult on users using (age >= 18) and (role != admin)\n\
         table users { id: uuid primary key default gen_uuid() email: text not null unique \
           role: Role default user age: int default 18 home: Address \
           created: timestamp default now() posts: posts -> many }\n\
         type Address { street: text city: text }\n\
         enum Role { admin, user }",
    );
    assert_eq!(a.to_json(), b.to_json());
    assert_eq!(SchemaIR::from_json(&a.to_json()).unwrap(), a);
}

#[test]
fn json_shape() {
    let ir = compile_ok("table t { id: uuid primary key }");
    let v: serde_json::Value = serde_json::from_str(&ir.to_json()).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["tables"][0]["columns"][0]["type"]["kind"], "builtin");
    assert_eq!(v["tables"][0]["columns"][0]["type"]["name"], "uuid");
}

#[test]
fn duplicate_declarations() {
    assert_eq!(errors("table a { id: int primary key } enum a { x }"), ["SDL200"]);
    assert_eq!(errors("table text { id: int primary key }"), ["SDL200"]);
    assert_eq!(
        errors("table t { id: int primary key } index i on t (id) index i on t (id)"),
        ["SDL200"]
    );
}

#[test]
fn duplicate_members_and_variants() {
    assert_eq!(errors("table t { id: int primary key id: int }"), ["SDL201"]);
    assert_eq!(errors("enum E { a, b, a }"), ["SDL204"]);
    assert_eq!(errors("type T { a: int a: int }"), ["SDL205"]);
}

#[test]
fn type_resolution_errors() {
    assert_eq!(errors("table t { id: int primary key x: nope }"), ["SDL202"]);
    assert_eq!(errors("table u { id: int primary key } table t { id: int primary key x: u }"), ["SDL202"]);
    assert_eq!(errors("table t { id: int primary key r: ghost -> one }"), ["SDL203"]);
    assert_eq!(errors("enum E { a } table t { id: int primary key r: E -> one }"), ["SDL203"]);
}

#[test]
fn composite_cycles() {
    assert_eq!(errors("type A { b: B } type B { a: A }").len(), 2);
    assert_eq!(errors("type A { a: A }"), ["SDL212"]);
}

#[test]
fn modifier_conflicts() {
    assert_eq!(errors("table t { id: int primary key null }"), ["SDL207"]);
    assert_eq!(errors("table t { id: int primary key x: int not null null }"), ["SDL207"]);
    assert_eq!(errors("table t { id: int primary key unique unique }"), ["SDL207"]);
    assert_eq!(errors("table t { id: int primary key default 1 default 2 }"), ["SDL207"]);
}

#[test]
fn default_checking() {
    assert_eq!(errors("table t { id: int primary key x: int default \"a\" }"), ["SDL208"]);
    assert_eq!(errors("enum E { a } table t { id: int primary key x: E default b }"), ["SDL210"]);
    assert_eq!(errors("table t { id: int primary key x: int default nope() }"), ["SDL209"]);
    assert_eq!(errors("table t { id: int primary key x: timestamp default now(1) }"), ["SDL209"]);
    assert_eq!(errors("type A { s: text } table t { id: int primary key x: A default 1 }"), ["SDL208"]);
    // valid: literal widening and string literals for text-like columns
    compile_ok("table t { id: int primary key f: float default 1  d: date default \"2026-01-01\" }");
}

#[test]
fn index_checking() {
    assert_eq!(errors("index i on ghost (a)"), ["SDL206"]);
    assert_eq!(errors("table t { id: int primary key } index i on t (nope)"), ["SDL206"]);
    assert_eq!(errors("table t { id: int primary key } index i on t (id, id)"), ["SDL206"]);
    // relationships are not columns
    assert_eq!(
        errors("table u { id: int primary key } table t { id: int primary key r: u -> one } index i on t (r)"),
        ["SDL206"]
    );
}

#[test]
fn constraint_checking() {
    assert_eq!(errors("table t { id: int primary key } constraint c on t using id + 1"), ["SDL211"]);
    assert_eq!(errors("table t { id: int primary key } constraint c on t using nope > 1"), ["SDL210"]);
    assert_eq!(errors("table t { id: int primary key } constraint c on t using id == \"a\""), ["SDL211"]);
    assert_eq!(errors("table t { id: int primary key } constraint c on ghost using id > 1"), ["SDL206"]);
    assert_eq!(
        errors("enum E { a } table t { id: int primary key e: E } constraint c on t using e < a"),
        ["SDL211"]
    );
}

#[test]
fn warns_without_primary_key_but_still_compiles() {
    let (ir, d) = compile("table t { x: int }");
    assert!(ir.is_some());
    assert!(d.iter().any(|x| x.code == "SDL220"));
}

#[test]
fn syntax_errors_skip_semantic_analysis() {
    let (ir, d) = compile("table t { x int } table u { y: nope }");
    assert!(ir.is_none());
    assert!(d.iter().all(|x| x.code.starts_with("SDL1")), "{d:?}");
}

#[test]
fn foreign_keys_resolve() {
    let ir = compile_ok(
        "table users { id: uuid primary key  email: text unique }
         table posts {
             id: uuid primary key
             author_id: uuid not null references users
             editor_email: text references users(email)
             parent_id: uuid references posts
         }",
    );
    let posts = ir.table("posts").unwrap();
    let fk = |c: &str| posts.column(c).unwrap().references.clone().unwrap();
    assert_eq!(fk("author_id"), ForeignKeyIR { table: "users".into(), column: "id".into(), ..Default::default() });
    assert_eq!(fk("editor_email"), ForeignKeyIR { table: "users".into(), column: "email".into(), ..Default::default() });
    assert_eq!(fk("parent_id").table, "posts"); // self-reference
    assert!(posts.column("id").unwrap().references.is_none());
    assert_eq!(SchemaIR::from_json(&ir.to_json()).unwrap(), ir);
}

#[test]
fn foreign_key_errors() {
    let u = "table u { id: uuid primary key  name: text } ";
    assert_eq!(errors("table t { id: int primary key r: int references ghost }"), ["SDL230"]);
    assert_eq!(errors(&format!("{u} table t {{ id: int primary key r: uuid references u(nope) }}")), ["SDL231"]);
    // not unique
    assert_eq!(errors(&format!("{u} table t {{ id: int primary key r: text references u(name) }}")), ["SDL231"]);
    // no primary key to default to / composite primary key
    assert_eq!(errors("table u { a: int } table t { id: int primary key r: int references u }"), ["SDL231"]);
    assert_eq!(
        errors("table u { a: int primary key b: int primary key } table t { id: int primary key r: int references u }"),
        ["SDL231"]
    );
    // type mismatch, duplicate modifier
    assert_eq!(errors(&format!("{u} table t {{ id: int primary key r: int references u }}")), ["SDL232"]);
    assert_eq!(errors(&format!("{u} table t {{ id: int primary key r: uuid references u references u }}")), ["SDL207"]);
}

#[test]
fn referential_actions() {
    let ir = compile_ok(
        "table u { id: uuid primary key }
         table t {
             id: uuid primary key
             a: uuid not null references u on delete cascade
             b: uuid references u on delete set null on update restrict
             c: uuid references u on update no action
             d: uuid references u
         }",
    );
    let t = ir.table("t").unwrap();
    let fk = |c: &str| t.column(c).unwrap().references.clone().unwrap();
    assert_eq!(fk("a").on_delete, ReferentialAction::Cascade);
    assert_eq!(fk("a").on_update, ReferentialAction::NoAction);
    assert_eq!(fk("b").on_delete, ReferentialAction::SetNull);
    assert_eq!(fk("b").on_update, ReferentialAction::Restrict);
    assert_eq!(fk("d"), ForeignKeyIR { table: "u".into(), column: "id".into(), ..Default::default() });
    assert_eq!(SchemaIR::from_json(&ir.to_json()).unwrap(), ir);
}

#[test]
fn referential_action_errors() {
    let u = "table u { id: uuid primary key } ";
    // set null on a non-nullable column
    assert_eq!(errors(&format!("{u} table t {{ id: uuid primary key r: uuid not null references u on delete set null }}")), ["SDL233"]);
    // duplicate clause
    let (_, d) = compile(&format!("{u} table t {{ id: uuid primary key r: uuid references u on delete cascade on delete restrict }}"));
    assert!(d.iter().any(|x| x.code == "SDL207"));
    // unknown action is a syntax error
    let (_, d) = compile(&format!("{u} table t {{ id: uuid primary key r: uuid references u on delete explode }}"));
    assert!(d.iter().any(|x| x.code == "SDL100"));
    // a column literally named `on` still parses
    compile_ok("table t { id: int primary key on: int }");
}

#[test]
fn is_null_function() {
    compile_ok("table t { id: int primary key x: int } constraint c on t using is_null(x) or id > 0");
}

#[test]
fn is_null_arity_is_checked() {
    assert_eq!(errors("table t { id: int primary key } constraint c on t using is_null()"), ["SDL209"]);
}


// ---- widened language ------------------------------------------------------ //

fn users(cols: &str) -> String { format!("table t {{ id: int primary key {cols} }}") }

fn default_of(cols: &str, col: &str) -> Option<ExprIR> {
    let ir = compile_ok(&users(cols));
    ir.table("t").unwrap().column(col).unwrap().default.clone()
}

fn constraint_of(cols: &str, expr: &str) -> ExprIR {
    let ir = compile_ok(&format!("{} constraint k on t using {expr}", users(cols)));
    ir.table("t").unwrap().constraints[0].expr.clone()
}

#[test]
fn negative_and_decimal_literals() {
    assert_eq!(default_of("n: int default -5", "n"), Some(ExprIR::Number { value: -5 }));
    assert_eq!(default_of("n: bigint default -9223372036854775808", "n"), Some(ExprIR::Number { value: i64::MIN }));
    assert_eq!(default_of("f: float default 1.5", "f"), Some(ExprIR::Decimal { value: "1.5".into() }));
    assert_eq!(default_of("f: float default -0.25", "f"), Some(ExprIR::Decimal { value: "-0.25".into() }));
    assert_eq!(default_of("d: decimal(10,2) default 19.99", "d"), Some(ExprIR::Decimal { value: "19.99".into() }));
    // decimal text is kept exactly, trailing zeros included
    assert_eq!(default_of("d: decimal(10,2) default 1.50", "d"), Some(ExprIR::Decimal { value: "1.50".into() }));
    // an integer literal fits a decimal column; a fraction does not fit an integer one
    assert!(default_of("d: decimal default 3", "d").is_some());
    assert_eq!(errors(&users("n: int default 1.5")), ["SDL208"]);
    assert_eq!(errors(&users("n: bigint default 9223372036854775808")), ["SDL002"]);
    // subtraction vs negation
    let e = constraint_of("a: int", "a - 1 > -1");
    let ExprIR::Binary { lhs, rhs, .. } = e else { panic!() };
    assert!(matches!(*lhs, ExprIR::Binary { op: BinaryOp::Sub, .. }));
    assert_eq!(*rhs, ExprIR::Number { value: -1 });
    let e = constraint_of("a: int", "a -1 > 0"); // `a -1` is a minus b, not two operands
    assert!(matches!(e, ExprIR::Binary { op: BinaryOp::Gt, .. }));
    assert!(matches!(constraint_of("a: int", "a - -1 > 0"), ExprIR::Binary { .. }));
}

#[test]
fn not_is_null_and_in() {
    // precedence: `not` is looser than comparison, tighter than and/or
    let e = constraint_of("a: int  b: int", "not a == b");
    let ExprIR::Not { expr } = e else { panic!("{e:?}") };
    assert!(matches!(*expr, ExprIR::Binary { op: BinaryOp::Eq, .. }));
    let e = constraint_of("a: bool  b: bool", "not a and b");
    let ExprIR::Binary { op: BinaryOp::And, lhs, .. } = e else { panic!() };
    assert!(matches!(*lhs, ExprIR::Not { .. }));
    assert!(matches!(constraint_of("a: bool", "not not a"), ExprIR::Not { .. }));

    // is [not] null, and the function spelling is the same thing
    assert_eq!(
        constraint_of("a: int", "a is null"),
        ExprIR::IsNull { expr: Box::new(ExprIR::Column { name: "a".into() }), negated: false }
    );
    assert_eq!(constraint_of("a: int", "is_null(a)"), constraint_of("a: int", "a is null"));
    assert!(matches!(constraint_of("a: int", "a is not null"), ExprIR::IsNull { negated: true, .. }));
    assert!(matches!(constraint_of("a: int  b: bool", "a is null or b"), ExprIR::Binary { op: BinaryOp::Or, .. }));

    // in / not in, with enum variants resolved from the left side
    let ir = compile_ok(
        "enum Role { admin, user, guest } table t { id: int primary key  role: Role  n: int }
         constraint a on t using role in (admin, user)
         constraint b on t using n not in (1, 2, 3)
         constraint c on t using not n in (1)",
    );
    let cs = &ir.table("t").unwrap().constraints;
    assert!(matches!(&cs[0].expr, ExprIR::In { list, negated: false, .. } if list.len() == 2
        && matches!(&list[0], ExprIR::EnumVariant { variant, .. } if variant == "admin")));
    assert!(matches!(&cs[1].expr, ExprIR::In { list, negated: true, .. } if list.len() == 3));
    assert!(matches!(&cs[2].expr, ExprIR::Not { expr } if matches!(**expr, ExprIR::In { .. })));

    // a column named like an operator word still works as a member name
    compile_ok("table t { id: int primary key  in: int  is: int  not: int }");
    compile_ok("table t { id: int primary key  a: int default 1  in: int }");
}

#[test]
fn not_is_null_and_in_type_errors() {
    assert_eq!(errors(&format!("{} constraint k on t using not a", users("a: int"))), ["SDL211"]);
    assert_eq!(errors(&format!("{} constraint k on t using a in (\"x\")", users("a: int"))), ["SDL211"]);
    assert_eq!(errors(&format!("{} constraint k on t using a in (nope)", users("a: int"))), ["SDL210"]);
    // syntax: `is` needs `null`; an `in` list needs elements
    let (_, d) = compile(&format!("{} constraint k on t using a is 5", users("a: int")));
    assert!(d.iter().any(|x| x.code == "SDL100"));
    let (_, d) = compile(&format!("{} constraint k on t using a in ()", users("a: int")));
    assert!(d.iter().any(|x| x.code == "SDL100"));
}

#[test]
fn new_types_and_parameters() {
    let ir = compile_ok(&users(
        "a: smallint  b: real  c: timestamp_naive  d: varchar(255)  e: char(3)  f: decimal(10,2)  g: decimal(8)  h: numeric(5,2)  i: numeric",
    ));
    let t = ir.table("t").unwrap();
    let ty = |c: &str| t.column(c).unwrap().ty.clone();
    assert_eq!(ty("a"), TypeIR::Builtin(Builtin::SmallInt));
    assert_eq!(ty("b"), TypeIR::Builtin(Builtin::Real));
    assert_eq!(ty("c"), TypeIR::Builtin(Builtin::TimestampNaive));
    assert_eq!(ty("d"), TypeIR::Builtin(Builtin::Varchar(255)));
    assert_eq!(ty("e"), TypeIR::Builtin(Builtin::Char(3)));
    assert_eq!(ty("f"), TypeIR::Builtin(Builtin::Numeric(10, 2)));
    assert_eq!(ty("g"), TypeIR::Builtin(Builtin::Numeric(8, 0)));
    assert_eq!(ty("h"), TypeIR::Builtin(Builtin::Numeric(5, 2)));
    assert_eq!(ty("i"), TypeIR::Builtin(Builtin::Decimal));
    // JSON shape for the parameterised ones
    let v: serde_json::Value = serde_json::from_str(&ir.to_json()).unwrap();
    let cols = &v["tables"][0]["columns"];
    assert_eq!(cols[4]["type"], serde_json::json!({"kind": "builtin", "name": {"varchar": 255}}));
    assert_eq!(cols[6]["type"], serde_json::json!({"kind": "builtin", "name": {"numeric": [10, 2]}}));
    assert_eq!(cols[3]["type"]["name"], "timestamp_naive");
    assert_eq!(SchemaIR::from_json(&ir.to_json()).unwrap(), ir);
}

#[test]
fn type_parameter_errors() {
    for (cols, code) in [
        ("x: text(5)", "SDL240"),
        ("x: uuid(1)", "SDL240"),
        ("x: varchar", "SDL241"),
        ("x: char", "SDL241"),
        ("x: varchar(0)", "SDL241"),
        ("x: varchar(1, 2)", "SDL241"),
        ("x: varchar(99999999)", "SDL241"),
        ("x: decimal(0)", "SDL241"),
        ("x: decimal(2, 5)", "SDL241"),
        ("x: decimal(1, 2, 3)", "SDL241"),
    ] {
        assert_eq!(errors(&users(cols)), [code], "{cols}");
    }
    // declarations may not reuse the new type names
    assert_eq!(errors("table varchar { id: int primary key }"), ["SDL200"]);
    assert_eq!(errors("enum real { a }"), ["SDL200"]);
    // relationships take no parameters
    let (_, d) = compile("table u { id: int primary key } table t { id: int primary key  r: u(1) -> one }");
    assert!(d.iter().any(|x| x.code == "SDL100"));
    // a parameterised type inside a composite type
    compile_ok("type T { s: varchar(20)  n: decimal(6,2) } table t { id: int primary key  x: T }");
}

#[test]
fn text_like_and_numeric_families_mix() {
    // varchar/char compare and combine with text; functions accept them
    compile_ok(&format!(
        "{} constraint k on t using length(v) > 0 and lower(v) == \"x\" and c == v and coalesce(v, \"d\") != \"\" and trim(c) != \"\"",
        users("v: varchar(20)  c: char(3)")
    ));
    // numeric families mix; a numeric(p,s) behaves like decimal
    compile_ok(&format!(
        "{} constraint k on t using a + b * c > 0 and round(d) >= 0 and abs(a) < 5",
        users("a: smallint  b: real  c: decimal(10,2)  d: float")
    ));
    // ...but text and numbers still do not
    assert_eq!(errors(&format!("{} constraint k on t using v == 1", users("v: varchar(5)"))), ["SDL211"]);
    // foreign keys need the same parameterised type on both sides
    let ok = "table u { id: varchar(10) primary key } table t { id: int primary key  r: varchar(10) references u }";
    compile_ok(ok);
    assert_eq!(
        errors("table u { id: varchar(10) primary key } table t { id: int primary key  r: varchar(20) references u }"),
        ["SDL232"]
    );
}

#[test]
fn new_functions() {
    assert!(matches!(default_of("d: date default today()", "d"), Some(ExprIR::Call { func, .. }) if func == "today"));
    compile_ok(&format!(
        "{} constraint k on t using round(x) > 0 and nullif(a, 0) > 1 and trim(s) != \"\"",
        users("x: float  a: int  s: text")
    ));
    assert_eq!(errors(&format!("{} constraint k on t using trim(a) == \"\"", users("a: int"))), ["SDL209"]);
    assert_eq!(errors(&format!("{} constraint k on t using round(s) > 0", users("s: text"))), ["SDL209"]);
    assert_eq!(errors(&format!("{} constraint k on t using nullif(a) > 0", users("a: int"))), ["SDL209"]);
    assert_eq!(errors(&users("d: date default today(1)")), ["SDL209"]);
}

#[test]
fn every_widened_feature_round_trips_through_print() {
    let src = r#"
        enum Role { admin, user }
        table t {
            id: int primary key
            a: smallint default -3
            b: real default 0.5
            c: timestamp_naive
            d: varchar(255) not null default "x"
            e: char(3)
            f: decimal(10,2) default -19.99
            g: decimal(8)
            h: date default today()
            r: Role
            n: int
        }
        constraint c1 on t using not n == 1 and (n is not null or a is null)
        constraint c2 on t using r in (admin, user) and n not in (1, -2, 3)
        constraint c3 on t using not (n > 1 or n < -5)
        constraint c4 on t using round(f) >= 0 and length(trim(d)) > 0 and nullif(n, 0) is not null
        constraint c5 on t using (n is null) == (a is null)
        constraint c6 on t using n - -1 > 0 and a - (n - 1) < 2
    "#;
    let (ir, d) = compile(src);
    let ir = ir.unwrap_or_else(|| panic!("{d:?}"));
    let printed = to_sdl(&ir).unwrap();
    let (again, d2) = compile(&printed);
    assert_eq!(again.unwrap_or_else(|| panic!("{d2:?}\n{printed}")), ir, "{printed}");
    assert_eq!(to_sdl(&compile(&printed).0.unwrap()).unwrap(), printed, "idempotent");
    for expected in [
        "d: varchar(255) not null default \"x\"",
        "f: decimal(10,2) default -19.99",
        "g: decimal(8)",
        "c: timestamp_naive",
        "n not in (1, -2, 3)",
        "not (n > 1 or n < -5)",
        "n is null == (a is null)",
        "a - (n - 1) < 2",
    ] {
        assert!(printed.contains(expected), "expected `{expected}` in:\n{printed}");
    }
}

#[test]
fn date_and_timestamp_types_mix_like_postgres() {
    compile_ok(&users("a: timestamp_naive default now()  b: timestamp default today()  c: date default now()"));
    compile_ok(&format!(
        "{} constraint k on t using a <= now() and b >= today() and a < b",
        users("a: timestamp_naive  b: timestamp")
    ));
    // ...but a timestamp is still not a number or text
    assert_eq!(errors(&users("a: timestamp default 5")), ["SDL208"]);
    assert_eq!(errors(&format!("{} constraint k on t using a == 1", users("a: timestamp"))), ["SDL211"]);
}


// ---- serial, identity and sequences ---------------------------------------- //

#[test]
fn serial_types_are_integer_columns_with_a_generation() {
    let ir = compile_ok("table t { a: serial primary key  b: bigserial  c: smallserial unique  d: int }");
    let t = ir.table("t").unwrap();
    let col = |n: &str| t.column(n).unwrap();
    assert_eq!(col("a").ty, TypeIR::Builtin(Builtin::Int));
    assert_eq!(col("b").ty, TypeIR::Builtin(Builtin::BigInt));
    assert_eq!(col("c").ty, TypeIR::Builtin(Builtin::SmallInt));
    for n in ["a", "b", "c"] {
        assert_eq!(col(n).generated, Some(Generation::Serial), "{n}");
        assert!(!col(n).nullable, "serial implies NOT NULL: {n}");
        assert!(col(n).default.is_none());
    }
    assert_eq!(col("d").generated, None);
    assert!(col("d").nullable);
    // JSON: the generation is a plain field, the type stays the integer type
    let v: serde_json::Value = serde_json::from_str(&ir.to_json()).unwrap();
    assert_eq!(v["tables"][0]["columns"][0]["generated"], "serial");
    assert_eq!(v["tables"][0]["columns"][0]["type"]["name"], "int");
    assert_eq!(SchemaIR::from_json(&ir.to_json()).unwrap(), ir);
}

#[test]
fn identity_columns() {
    let ir = compile_ok("table t { a: bigint primary key generated always  b: int generated by default  c: int }");
    let t = ir.table("t").unwrap();
    assert_eq!(t.column("a").unwrap().generated, Some(Generation::Always));
    assert_eq!(t.column("b").unwrap().generated, Some(Generation::ByDefault));
    assert!(!t.column("b").unwrap().nullable);
    let v: serde_json::Value = serde_json::from_str(&ir.to_json()).unwrap();
    assert_eq!(v["tables"][0]["columns"][1]["generated"], "by_default");
}

#[test]
fn a_foreign_key_to_a_serial_key_is_a_plain_integer_column() {
    // the whole point of modelling serial as a property, not a type
    compile_ok("table u { id: serial primary key } table t { id: int primary key  u_id: int references u }");
    compile_ok("table u { id: bigserial primary key } table t { id: int primary key  u_id: bigint references u }");
    assert_eq!(
        errors("table u { id: bigserial primary key } table t { id: int primary key  u_id: int references u }"),
        ["SDL232"] // int vs bigint is still a mismatch
    );
}

#[test]
fn generated_column_errors() {
    assert_eq!(errors("table t { id: int primary key  a: text generated always }"), ["SDL251"]);
    assert_eq!(errors("table t { id: int primary key  a: uuid generated by default }"), ["SDL251"]);
    assert_eq!(errors("table t { id: int primary key  a: serial default 5 }"), ["SDL250"]);
    assert_eq!(errors("table t { id: int primary key  a: int generated always default 5 }"), ["SDL250"]);
    assert_eq!(errors("table t { id: int primary key  a: serial generated always }"), ["SDL250"]);
    assert_eq!(errors("table t { id: int primary key  a: serial null }"), ["SDL207"]);
    assert_eq!(errors("table t { id: int primary key  a: int generated always null }"), ["SDL207"]);
    // serial is a column spelling only
    assert_eq!(errors("type T { a: serial } table t { id: int primary key }"), ["SDL252"]);
    // and takes no parameters; the names are reserved
    assert_eq!(errors("table t { id: int primary key  a: serial(5) }"), ["SDL240"]);
    assert_eq!(errors("table serial { id: int primary key }"), ["SDL200"]);
    // `generated` needs `always` or `by default`
    let (_, d) = compile("table t { id: int primary key  a: int generated sometimes }");
    assert!(d.iter().any(|x| x.code == "SDL100"));
}

#[test]
fn sequences_resolve_postgres_defaults() {
    let ir = compile_ok(
        "sequence s1
         sequence s2 start 1000 increment 5 cycle cache 20
         sequence s3 increment -1
         sequence s4 min 10 max 99
         sequence s5 start -5 increment -2 min -100 max -1",
    );
    let s = |n: &str| ir.sequences.iter().find(|s| s.name == n).unwrap().clone();
    let seq = |name: &str, start, increment, min, max, cache, cycle| SequenceIR { name: name.into(), start, increment, min, max, cache, cycle };
    assert_eq!(s("s1"), seq("s1", 1, 1, 1, i64::MAX, 1, false));
    assert_eq!(s("s2"), seq("s2", 1000, 5, 1, i64::MAX, 20, true));
    assert_eq!(s("s3"), seq("s3", -1, -1, i64::MIN, -1, 1, false)); // descending starts at max
    assert_eq!(s("s4"), seq("s4", 10, 1, 10, 99, 1, false)); // start defaults to min
    assert_eq!(s("s5"), seq("s5", -5, -2, -100, -1, 1, false));
    // sorted by name, in the JSON too
    assert_eq!(ir.sequences.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["s1", "s2", "s3", "s4", "s5"]);
    assert_eq!(SchemaIR::from_json(&ir.to_json()).unwrap(), ir);
}

#[test]
fn sequence_errors() {
    assert_eq!(errors("sequence s increment 0"), ["SDL254"]);
    assert_eq!(errors("sequence s min 10 max 5"), ["SDL254"]);
    assert_eq!(errors("sequence s start 0"), ["SDL254"]); // below the default min of 1
    assert_eq!(errors("sequence s start 100 max 50"), ["SDL254"]);
    assert_eq!(errors("sequence s cache 0"), ["SDL254"]);
    assert_eq!(errors("sequence s start 1 start 2"), ["SDL253"]);
    assert_eq!(errors("sequence s cycle cycle"), ["SDL253"]);
    // a sequence and a table may not share a name
    assert_eq!(errors("sequence s table s { id: int primary key }"), ["SDL200"]);
}

#[test]
fn nextval_defaults() {
    let ir = compile_ok("sequence order_seq start 1000  table t { id: bigint primary key default nextval(order_seq)  n: int default nextval(order_seq) }");
    let t = ir.table("t").unwrap();
    assert_eq!(t.column("id").unwrap().default, Some(ExprIR::NextVal { sequence: "order_seq".into() }));
    assert!(t.column("n").unwrap().default.is_some(), "a bigint default fits an int column like any numeric");
    // must name a declared sequence, and only one
    assert_eq!(errors("table t { id: int primary key  a: int default nextval(nope) }"), ["SDL209"]);
    assert_eq!(errors("sequence s table t { id: int primary key  a: int default nextval(s, s) }"), ["SDL209"]);
    assert_eq!(errors("sequence s table t { id: int primary key  a: int default nextval(1) }"), ["SDL209"]);
    assert_eq!(errors("sequence s table t { id: int primary key  a: text default nextval(s) }"), ["SDL208"]);
    // a sequence is not a type, and shares the namespace of tables
    assert_eq!(errors("sequence s table t { id: int primary key  a: s }"), ["SDL202"]);
    assert_eq!(errors("sequence t table t { id: int primary key }"), ["SDL200"]);
}

#[test]
fn serial_identity_and_sequences_round_trip_through_print() {
    let src = r#"
        sequence order_seq start 1000 increment 5 cycle cache 10
        sequence down_seq increment -2 min -50 max -1 start -3
        sequence plain_seq
        table t {
            a: serial primary key
            b: bigserial unique
            c: smallserial
            d: bigint generated always
            e: int generated by default
            f: bigint default nextval(order_seq)
            g: int not null
        }
    "#;
    let (ir, d) = compile(src);
    let ir = ir.unwrap_or_else(|| panic!("{d:?}"));
    let printed = to_sdl(&ir).unwrap();
    let (again, d2) = compile(&printed);
    assert_eq!(again.unwrap_or_else(|| panic!("{d2:?}\n{printed}")), ir, "{printed}");
    assert_eq!(to_sdl(&compile(&printed).0.unwrap()).unwrap(), printed, "idempotent");
    for expected in [
        "sequence order_seq start 1000 increment 5 cache 10 cycle",
        "sequence plain_seq\n",
        "a: serial primary key",
        "b: bigserial unique",
        "c: smallserial\n",
        "d: bigint generated always",
        "e: int generated by default",
        "f: bigint default nextval(order_seq)",
        "g: int not null",
    ] {
        assert!(printed.contains(expected), "expected `{expected}` in:\n{printed}");
    }
    assert!(!printed.contains("a: serial primary key not null"), "implied NOT NULL is not printed");
}

const WITH_VIEWS: &str = "
table users { id: serial primary key  email: text not null  age: int  note: text }
view adults on users (id, email) where age >= 18
view everyone on users (email, age)
";

#[test]
fn views_resolve_to_the_table_they_read() {
    let ir = compile_ok(WITH_VIEWS);
    assert_eq!(ir.tables.len(), 1, "a view is not a table in the schema");
    let names: Vec<_> = ir.views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["adults", "everyone"]);
    let adults = ir.view("adults").unwrap();
    assert_eq!((adults.from.as_str(), adults.columns.as_slice()), ("users", ["id".to_string(), "email".to_string()].as_slice()));
    assert!(adults.filter.is_some() && ir.view("everyone").unwrap().filter.is_none());

    // what a reader sees: the chosen columns, nothing that belongs to the table
    let t = adults.as_table(&ir).unwrap();
    assert!(t.view && t.columns.len() == 2);
    let id = t.column("id").unwrap();
    assert!(!id.primary_key && id.generated.is_none() && !id.nullable);
    assert!(ir.view("everyone").unwrap().as_table(&ir).unwrap().column("age").unwrap().nullable);
    let q = ir.with_views_as_tables();
    assert!(q.views.is_empty() && q.table("adults").is_some_and(|t| t.view));
}

#[test]
fn views_print_and_compile_back_to_the_same_schema() {
    let ir = compile_ok(WITH_VIEWS);
    let printed = to_sdl(&ir).unwrap();
    assert!(printed.contains("view adults on users (id, email) where age >= 18"), "{printed}");
    assert!(printed.contains("view everyone on users (email, age)\n") || printed.trim_end().ends_with("view everyone on users (email, age)"), "{printed}");
    assert_eq!(compile_ok(&printed), ir);
    // a schema with no views serializes as before
    assert!(!compile_ok("table t { id: int primary key }").to_json().contains("views"));
}

#[test]
fn view_mistakes() {
    let t = "table users { id: serial primary key  email: text  age: int }\n";
    assert_eq!(errors(&format!("{t}view v on nope (id)")), ["SDL206"]);
    assert_eq!(errors(&format!("{t}view v on users (ghost)")), ["SDL206"]);
    assert_eq!(errors(&format!("{t}view v on users (id, id)")), ["SDL206"]);
    assert_eq!(errors(&format!("{t}view v on users (id) where age")), ["SDL211"]);
    assert_eq!(errors(&format!("{t}view v on users (id) where ghost > 1")).len(), 1);
    assert_eq!(errors(&format!("{t}view users on users (id)")), ["SDL200"]);
    assert_eq!(errors(&format!("{t}view a on users (id)\nview b on a (id)")), ["SDL213"]);
    assert_eq!(errors(&format!("{t}view v on users (id)\nview v on users (email)")), ["SDL200"]);
    let (_, d) = compile(&format!("{t}view v on users"));
    assert!(d.iter().any(|x| x.severity == certo_diagnostics::Severity::Error), "a view needs a column list");
}
