use super::parse;
use certo_ast::expr::{Expr, Lit, BinOp};
use certo_ast::decl::Decl;

fn ok(src: &str) -> certo_ast::module::Module {
    parse(src).unwrap_or_else(|errs| {
        panic!("parse failed:\n{}", errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n"))
    })
}

fn err(src: &str) {
    assert!(parse(src).is_err(), "expected parse error but got Ok");
}

// ------------------------------------------------------------------ //
// Module / import
// ------------------------------------------------------------------ //

#[test]
fn empty_module() {
    let m = ok("module MyApp");
    assert_eq!(m.path.segments.len(), 1);
    assert_eq!(m.path.segments[0].node, "MyApp");
}

#[test]
fn nested_module_path() {
    let m = ok("module MyApp.Orders.Processing");
    assert_eq!(m.path.segments.len(), 3);
}

#[test]
fn import_whole() {
    let m = ok("module A\nimport Stdlib.DateTime");
    assert_eq!(m.imports.len(), 1);
}

#[test]
fn import_named() {
    let m = ok("module A\nimport Stdlib.Collections.{ List, Map }");
    assert_eq!(m.imports.len(), 1);
    let names = match &m.imports[0].kind {
        certo_ast::module::ImportKind::Named(n) => n,
        _ => panic!("expected named import"),
    };
    assert_eq!(names.len(), 2);
    assert_eq!(names[0].name.node, "List");
    assert_eq!(names[1].name.node, "Map");
}

#[test]
fn import_aliased() {
    let m = ok("module A\nimport MyApp.Models.Order as O");
    match &m.imports[0].kind {
        certo_ast::module::ImportKind::Aliased(a) => assert_eq!(a.node, "O"),
        _ => panic!("expected aliased import"),
    }
}

// ------------------------------------------------------------------ //
// Declarations
// ------------------------------------------------------------------ //

#[test]
fn fn_decl_simple() {
    let m = ok("module A\nfn add(a: Int, b: Int): Int = a + b");
    assert_eq!(m.decls.len(), 1);
    match &m.decls[0].node {
        Decl::Fn(f) => {
            assert_eq!(f.name.node, "add");
            assert_eq!(f.params.len(), 2);
        }
        _ => panic!("expected fn decl"),
    }
}

#[test]
fn async_fn_decl() {
    let m = ok("module A\nasync fn fetchUser(id: UUID): Result<User, DbError> = todo()");
    match &m.decls[0].node {
        Decl::Fn(f) => assert!(f.is_async),
        _ => panic!(),
    }
}

#[test]
fn type_alias() {
    let m = ok("module A\ntype UserId = UUID");
    match &m.decls[0].node {
        Decl::Type(t) => assert_eq!(t.name.node, "UserId"),
        _ => panic!(),
    }
}

#[test]
fn sum_type() {
    let m = ok("module A\ntype Shape =\n    | Circle(radius: Float)\n    | Rectangle(width: Float, height: Float)");
    match &m.decls[0].node {
        Decl::Type(t) => match &t.body {
            certo_ast::decl::TypeBody::Sum(variants) => assert_eq!(variants.len(), 2),
            _ => panic!("expected sum type"),
        },
        _ => panic!(),
    }
}

#[test]
fn val_decl() {
    let m = ok("module A\nval name = \"Alice\"");
    assert!(matches!(m.decls[0].node, Decl::Val(_)));
}

// ------------------------------------------------------------------ //
// Expressions
// ------------------------------------------------------------------ //

#[test]
fn integer_literal() {
    let m = ok("module A\nval x = 42");
    match &m.decls[0].node {
        Decl::Val(v) => match &v.value.node {
            Expr::Lit { value: Lit::Int(42), .. } => {}
            other => panic!("unexpected: {other:?}"),
        },
        _ => panic!(),
    }
}

#[test]
fn addition() {
    let m = ok("module A\nval x = 1 + 2");
    match &m.decls[0].node {
        Decl::Val(v) => match &v.value.node {
            Expr::BinOp { op: BinOp::Add, .. } => {}
            other => panic!("{other:?}"),
        },
        _ => panic!(),
    }
}

#[test]
fn pipeline_expr() {
    let m = ok("module A\nval x = a |> f |> g");
    match &m.decls[0].node {
        Decl::Val(v) => assert!(matches!(v.value.node, Expr::Pipe { .. })),
        _ => panic!(),
    }
}

#[test]
fn pipeline_with_args() {
    // `xs |> List.filter(pred) |> List.map(f)` — each RHS is an App node
    let m = ok("module A\nval y = xs |> List.filter(pred) |> List.map(f)");
    match &m.decls[0].node {
        Decl::Val(v) => {
            // Outer node is Pipe (the second |>)
            assert!(matches!(v.value.node, Expr::Pipe { .. }));
            // RHS of outer pipe is an App (List.map(f))
            if let Expr::Pipe { right, .. } = &v.value.node {
                assert!(matches!(right.node, Expr::App { .. }),
                    "RHS of pipe with args should parse as App");
            }
        }
        _ => panic!(),
    }
}

#[test]
fn if_then_else() {
    let m = ok("module A\nval x = if true then 1 else 2");
    match &m.decls[0].node {
        Decl::Val(v) => assert!(matches!(v.value.node, Expr::If { .. })),
        _ => panic!(),
    }
}

#[test]
fn match_expr() {
    let m = ok("module A\nval x = match y {\n    Ok(v) => v\n    Err(e) => 0\n}");
    match &m.decls[0].node {
        Decl::Val(v) => assert!(matches!(v.value.node, Expr::Match { .. })),
        _ => panic!(),
    }
}

#[test]
fn match_guard() {
    let src = "module A\nfn f(n: Int): Text = match n {\n    x if x > 0 => \"pos\"\n    _ => \"other\"\n}";
    let m = ok(src);
    match &m.decls[0].node {
        Decl::Fn(f) => {
            if let Expr::Match { arms, .. } = &f.body.as_ref().unwrap().node {
                assert!(arms[0].guard.is_some(), "first arm should have a guard");
                assert!(arms[1].guard.is_none(), "wildcard arm should have no guard");
            } else {
                panic!("expected Match");
            }
        }
        _ => panic!(),
    }
}

#[test]
fn block_with_val() {
    let m = ok("module A\nfn f(): Int = { val x = 1 x }");
    match &m.decls[0].node {
        Decl::Fn(f) => assert!(matches!(f.body.as_ref().unwrap().node, Expr::Block { .. })),
        _ => panic!(),
    }
}

// ------------------------------------------------------------------ //
// Error recovery
// ------------------------------------------------------------------ //

#[test]
fn missing_module_decl_is_error() {
    err("fn add(a: Int): Int = a");
}

// ------------------------------------------------------------------ //
// Phase 2 — validator feature declarations
// ------------------------------------------------------------------ //

#[test]
fn constraint_decl_simple() {
    let m = ok("module A\nconstraint WithinCredit = order.total <= customer.availableCredit");
    match &m.decls[0].node {
        Decl::Constraint(c) => {
            assert_eq!(c.name.node, "WithinCredit");
            assert!(!c.is_pub);
        }
        _ => panic!("expected Constraint decl"),
    }
}

#[test]
fn constraint_decl_pub() {
    let m = ok("module A\npub constraint UserIsAdmin = user.role == Admin");
    match &m.decls[0].node {
        Decl::Constraint(c) => {
            assert_eq!(c.name.node, "UserIsAdmin");
            assert!(c.is_pub);
        }
        _ => panic!("expected Constraint decl"),
    }
}

#[test]
fn temporal_decl_simple() {
    let m = ok("module A\ntemporal GracePeriod = Duration.hours(48)");
    match &m.decls[0].node {
        Decl::Temporal(t) => assert_eq!(t.name.node, "GracePeriod"),
        _ => panic!("expected Temporal decl"),
    }
}

#[test]
fn temporal_decl_pub() {
    let m = ok("module A\npub temporal VoidWindow = Duration.days(30)");
    match &m.decls[0].node {
        Decl::Temporal(t) => {
            assert_eq!(t.name.node, "VoidWindow");
            assert!(t.is_pub);
        }
        _ => panic!("expected Temporal decl"),
    }
}

#[test]
fn validator_decl_minimal() {
    let m = ok("module A\nvalidator V for Order errors OrderError {\n    rule r { require true else Err(OrderError.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            assert_eq!(v.name.node, "V");
            assert!(!v.is_pub);
            assert_eq!(v.rules.len(), 1);
            assert_eq!(v.rules[0].name.node, "r");
            assert!(v.trigger.is_none());
            assert!(v.context.is_empty());
        }
        _ => panic!("expected Validator decl"),
    }
}

#[test]
fn validator_decl_pub() {
    let m = ok("module A\npub validator V for Order errors OrderError {\n    rule r { require true else Err(OrderError.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => assert!(v.is_pub),
        _ => panic!(),
    }
}

#[test]
fn validator_decl_trigger_insert() {
    let m = ok("module A\nvalidator V for Order errors OE\n    trigger on Insert\n{\n    rule r { require true else Err(OE.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let t = v.trigger.as_ref().expect("trigger missing");
            assert!(matches!(t.op, certo_ast::decl::TriggerOp::Insert));
            assert!(t.condition.is_none());
        }
        _ => panic!(),
    }
}

#[test]
fn validator_decl_trigger_update_when_eq() {
    let m = ok("module A\nvalidator V for Order errors OE\n    trigger on Update when status == Submitted\n{\n    rule r { require true else Err(OE.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let t = v.trigger.as_ref().expect("trigger missing");
            assert!(matches!(t.op, certo_ast::decl::TriggerOp::Update));
            let cond = t.condition.as_ref().expect("condition missing");
            assert_eq!(cond.field.node, "status");
            assert!(matches!(cond.op, certo_ast::decl::TriggerCondOp::Eq));
        }
        _ => panic!(),
    }
}

#[test]
fn validator_decl_trigger_update_when_neq() {
    let m = ok("module A\nvalidator V for Order errors OE\n    trigger on Update when status != OLD.status\n{\n    rule r { require true else Err(OE.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let cond = v.trigger.as_ref().unwrap().condition.as_ref().unwrap();
            assert!(matches!(cond.op, certo_ast::decl::TriggerCondOp::NotEq));
        }
        _ => panic!(),
    }
}

#[test]
fn validator_decl_context_block() {
    let m = ok("module A\nvalidator V for Order errors OE {\n    context {\n        customer: Customer\n        user: User\n    }\n    rule r { require true else Err(OE.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            assert_eq!(v.context.len(), 2);
            assert_eq!(v.context[0].name.node, "customer");
            assert_eq!(v.context[1].name.node, "user");
            assert!(v.context[0].loaded_by.is_none());
        }
        _ => panic!(),
    }
}

#[test]
fn validator_decl_context_loaded_by() {
    let m = ok("module A\nvalidator V for Order errors OE {\n    context {\n        customer: Customer loaded by db.customers.find(order.customerId)\n    }\n    rule r { require true else Err(OE.X) }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            assert!(v.context[0].loaded_by.is_some(), "loaded_by should be present");
        }
        _ => panic!(),
    }
}

#[test]
fn validator_rule_multiple_afters() {
    let m = ok("module A\nvalidator V for Order errors OE {\n    rule a { require true else Err(OE.X) }\n    rule b {\n        after a\n        require true\n        else Err(OE.X)\n    }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let b = &v.rules[1];
            assert_eq!(b.after.len(), 1);
            assert_eq!(b.after[0].node, "a");
        }
        _ => panic!(),
    }
}

#[test]
fn validator_rule_overrides_and_priority() {
    let m = ok("module A\nvalidator V for Order errors OE {\n    rule a { require true else Err(OE.X) }\n    rule b {\n        overrides a\n        priority 100\n        require true\n        else Err(OE.X)\n    }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let b = &v.rules[1];
            assert_eq!(b.overrides.as_ref().unwrap().node, "a");
            assert_eq!(b.priority, Some(100));
        }
        _ => panic!(),
    }
}

#[test]
fn validator_rule_all_modifiers_any_order() {
    // after, overrides, priority can appear in any order
    let m = ok("module A\nvalidator V for Order errors OE {\n    rule base { require true else Err(OE.X) }\n    rule b {\n        priority 10\n        after base\n        overrides base\n        require true\n        else Err(OE.X)\n    }\n}");
    match &m.decls[0].node {
        Decl::Validator(v) => {
            let b = &v.rules[1];
            assert_eq!(b.priority, Some(10));
            assert_eq!(b.after.len(), 1);
            assert!(b.overrides.is_some());
        }
        _ => panic!(),
    }
}

#[test]
fn rule_test_decl_pass() {
    let m = ok("module A\nruleTest OrderSubmit.customer_active \"passes for active\" {\n    entity: Order { id: 1 }\n    context: defaultCtx\n    expect: pass\n}");
    match &m.decls[0].node {
        Decl::RuleTest(rt) => {
            assert_eq!(rt.validator.len(), 2);
            assert_eq!(rt.validator[0].node, "OrderSubmit");
            assert_eq!(rt.validator[1].node, "customer_active");
            assert_eq!(rt.label, "passes for active");
            assert!(matches!(rt.expect, certo_ast::decl::TestExpectation::Pass));
        }
        _ => panic!("expected RuleTest decl"),
    }
}

#[test]
fn rule_test_decl_fail() {
    let m = ok("module A\nruleTest OrderSubmit.customer_active \"fails when inactive\" {\n    entity: Order { id: 1 }\n    context: defaultCtx\n    expect: fail\n}");
    match &m.decls[0].node {
        Decl::RuleTest(rt) => {
            assert!(matches!(rt.expect, certo_ast::decl::TestExpectation::Fail { with: None }));
        }
        _ => panic!(),
    }
}

#[test]
fn validator_test_decl_fail_with() {
    let m = ok("module A\nvalidatorTest OrderSubmit \"label\" {\n    entity: validOrder\n    context: defaultCtx\n    expect: fail with OrderError.CustomerNotActive\n}");
    match &m.decls[0].node {
        Decl::ValidatorTest(vt) => {
            assert_eq!(vt.validator.node, "OrderSubmit");
            assert!(matches!(&vt.expect, certo_ast::decl::TestExpectation::Fail { with: Some(_) }));
        }
        _ => panic!("expected ValidatorTest decl"),
    }
}

#[test]
fn dot_age_postfix() {
    let m = ok("module A\nval x = invoice.createdAt.age");
    match &m.decls[0].node {
        Decl::Val(v) => {
            assert!(matches!(v.value.node, Expr::Age { .. }), "expected Age expr, got {:?}", v.value.node);
        }
        _ => panic!(),
    }
}

#[test]
fn dot_age_in_expression() {
    // .age can appear in a comparison
    let m = ok("module A\nval x = invoice.createdAt.age < limit");
    match &m.decls[0].node {
        Decl::Val(v) => assert!(matches!(v.value.node, Expr::BinOp { op: BinOp::Lt, .. })),
        _ => panic!(),
    }
}

// Parse errors

#[test]
fn validator_missing_errors_keyword() {
    err("module A\nvalidator V for Order { rule r { require true else Err(OE.X) } }");
}

#[test]
fn validator_missing_for_keyword() {
    err("module A\nvalidator V errors OE for Order { }");
}

#[test]
fn rule_missing_else() {
    err("module A\nvalidator V for Order errors OE { rule r { require true } }");
}

#[test]
fn context_loaded_missing_by() {
    err("module A\nvalidator V for Order errors OE { context { customer: Customer loaded } rule r { require true else Err(OE.X) } }");
}

// ------------------------------------------------------------------ //
// Newline-aware statement boundaries
// ------------------------------------------------------------------ //

#[test]
fn statement_then_parenthesised_expr_is_two_statements() {
    // A `(` starting a new line must NOT bind to the previous statement as a
    // call. Here `spawn work(b)` then `(await t1) + (await t2)` are distinct.
    let m = ok("module A\nfn work(n: Int): Int = n\nfn run(a: Int, b: Int): Int = {\n\
        val t1 = spawn work(a)\n  val t2 = spawn work(b)\n  (await t1) + (await t2)\n}");
    let Decl::Fn(f) = &m.decls[1].node else { panic!("expected fn") };
    let Expr::Block { stmts, .. } = &f.body.as_ref().unwrap().node else { panic!("expected block body") };
    // Two `val`s plus the tail expression statement.
    assert_eq!(stmts.len(), 3, "expected 3 statements, got {}", stmts.len());
}

#[test]
fn multiline_arg_list_still_parses_as_call() {
    // A `(` on the SAME line as the callee is still a call, even if the args
    // span multiple lines.
    ok("module A\nfn add3(a: Int, b: Int, c: Int): Int = a + b + c\n\
        fn run(): Int = add3(\n  1,\n  2,\n  3\n)");
}

#[test]
fn match_on_nullary_constructors_parses() {
    // `match c { Red => … }` must read `{ … }` as the match body, not as a
    // trailing-lambda call on `c` (the arm `Red =>` looks like a lambda param).
    let m = ok("module A\ntype Color = | Red | Green | Blue\n\
        fn name(c: Color): Text = match c {\n  Red => \"r\"\n  Green => \"g\"\n  Blue => \"b\"\n}");
    let Decl::Fn(f) = &m.decls[1].node else { panic!("expected fn") };
    let Expr::Match { arms, .. } = &f.body.as_ref().unwrap().node else { panic!("expected match body") };
    assert_eq!(arms.len(), 3, "expected 3 match arms, got {}", arms.len());
}

#[test]
fn trailing_lambda_still_works_outside_match_scrutinee() {
    // The suppression must be scoped to the scrutinee only.
    ok("module A\nfn f(xs: List<Int>): List<Int> = List.map(xs) { x => x }");
}

// ------------------------------------------------------------------ //
// Migration operations (structured parsing)
// ------------------------------------------------------------------ //

fn only_migration(src: &str) -> certo_ast::decl::MigrationDecl {
    let m = ok(src);
    for d in m.decls {
        if let Decl::Migration(mig) = d.node { return mig; }
    }
    panic!("no migration declaration found");
}

#[test]
fn migration_create_table_with_modifiers() {
    use certo_ast::decl::MigrationOp;
    let mig = only_migration(
        "module A\nmigration \"init\" {\n\
           up { createTable users { id: UUID primaryKey, email: Text unique nullable, age: Int default 0 } }\n\
           down { dropTable users }\n\
         }");
    match &mig.up[0] {
        MigrationOp::CreateTable { name, columns, .. } => {
            assert_eq!(name, "users");
            assert_eq!(columns.len(), 3);
            assert!(columns[0].primary_key);
            assert!(columns[1].unique && columns[1].nullable);
            assert!(columns[2].default.is_some());
        }
        other => panic!("expected CreateTable, got {other:?}"),
    }
    assert!(matches!(&mig.down[0], MigrationOp::DropTable { name, .. } if name == "users"));
}

#[test]
fn migration_alter_table_ops() {
    use certo_ast::decl::{MigrationOp, AlterOp, FkAction};
    let mig = only_migration(
        "module A\nmigration \"m\" {\n\
           up { alterTable posts {\n\
                  addColumn authorId: UUID\n\
                  dropColumn legacy\n\
                  foreignKey authorId references users onDelete cascade\n\
                } }\n\
           down { alterTable posts { dropColumn authorId } }\n\
         }");
    let MigrationOp::AlterTable { name, ops, .. } = &mig.up[0] else { panic!("expected AlterTable") };
    assert_eq!(name, "posts");
    assert_eq!(ops.len(), 3);
    assert!(matches!(&ops[0], AlterOp::AddColumn { def } if def.name == "authorId"));
    assert!(matches!(&ops[1], AlterOp::DropColumn { name, .. } if name == "legacy"));
    assert!(matches!(&ops[2],
        AlterOp::AddForeignKey { column, references, on_delete: FkAction::Cascade, .. }
        if column == "authorId" && references == "users"));
}

#[test]
fn migration_index_ops() {
    use certo_ast::decl::MigrationOp;
    let mig = only_migration(
        "module A\nmigration \"idx\" {\n\
           up { createIndex users_email_idx on users [email, name] }\n\
           down { dropIndex users_email_idx }\n\
         }");
    let MigrationOp::CreateIndex { name, table, columns, .. } = &mig.up[0] else { panic!("expected CreateIndex") };
    assert_eq!(name, "users_email_idx");
    assert_eq!(table, "users");
    assert_eq!(columns, &vec!["email".to_string(), "name".to_string()]);
    assert!(matches!(&mig.down[0], MigrationOp::DropIndex { name, .. } if name == "users_email_idx"));
}

#[test]
fn migration_raw_sql_escape_hatch() {
    use certo_ast::decl::MigrationOp;
    let mig = only_migration(
        "module A\nmigration \"raw\" {\n\
           up { rawSql \"VACUUM FULL;\" }\n\
           down { rawSql \"-- noop\" }\n\
         }");
    assert!(matches!(&mig.up[0], MigrationOp::RawSql { sql, .. } if sql == "VACUUM FULL;"));
}

#[test]
fn migration_unknown_op_errors() {
    err("module A\nmigration \"x\" { up { frobnicate users } down { } }");
}
