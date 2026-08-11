use super::parse;
use certo_ast::expr::{Expr, Lit, BinOp};
use certo_ast::decl::Decl;
use certo_ast::span::S;

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
fn property_decl_without_params_still_parses() {
    // Backward compat: a bare `property "name" { .. }` (BACKLOG item 86's
    // typed params are optional) must keep parsing exactly as before.
    let m = ok("module A\nproperty \"reverse twice is identity\" { true }");
    assert_eq!(m.decls.len(), 1);
    match &m.decls[0].node {
        Decl::Property(p) => {
            assert_eq!(p.name, "reverse twice is identity");
            assert!(p.params.is_empty());
        }
        _ => panic!("expected property decl"),
    }
}

#[test]
fn property_decl_with_typed_params_parses() {
    let m = ok("module A\nproperty \"addition commutes\"(x: Int, y: Int) { x + y == y + x }");
    assert_eq!(m.decls.len(), 1);
    match &m.decls[0].node {
        Decl::Property(p) => {
            assert_eq!(p.name, "addition commutes");
            assert_eq!(p.params.len(), 2);
            assert_eq!(p.params[0].name.node, "x");
            assert_eq!(p.params[1].name.node, "y");
        }
        _ => panic!("expected property decl"),
    }
}

#[test]
fn row_bound_parses() {
    use certo_ast::types::Bound;
    let m = ok("module A\nfn getName<R: { name: Text }>(record: R): Text = record.name");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    assert_eq!(f.type_params.len(), 1);
    assert_eq!(f.type_params[0].name.node, "R");
    assert_eq!(f.type_params[0].bounds.len(), 1);
    let Bound::Row(row) = &f.type_params[0].bounds[0] else { panic!("expected a row bound") };
    assert_eq!(row.fields.len(), 1);
    assert_eq!(row.fields[0].name.node, "name");
}

#[test]
fn row_bound_with_multiple_fields_parses() {
    use certo_ast::types::Bound;
    let m = ok("module A\nfn f<R: { name: Text, age: Int }>(r: R): Text = r.name");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    let Bound::Row(row) = &f.type_params[0].bounds[0] else { panic!("expected a row bound") };
    assert_eq!(row.fields.len(), 2);
    assert_eq!(row.fields[1].name.node, "age");
}

#[test]
fn trait_bound_still_parses_as_before() {
    // Regression guard: the Bound enum change must not affect ordinary trait bounds.
    use certo_ast::types::Bound;
    let m = ok("module A\nfn f<T: Serializable>(x: T): Text = todo()");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    let Bound::Trait(tb) = &f.type_params[0].bounds[0] else { panic!("expected a trait bound") };
    assert_eq!(tb.name.segments[0].node, "Serializable");
}

#[test]
fn mixed_trait_and_row_bounds_parse() {
    use certo_ast::types::Bound;
    let m = ok("module A\nfn f<R: Serializable + { name: Text }>(r: R): Text = r.name");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    let bounds = &f.type_params[0].bounds;
    assert_eq!(bounds.len(), 2);
    assert!(matches!(&bounds[0], Bound::Trait(_)));
    assert!(matches!(&bounds[1], Bound::Row(_)));
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
fn export_annotation_sets_export_name() {
    let m = ok("module A\n@export(\"my_custom_add\")\npub fn addNumbers(a: Int, b: Int): Int = a + b");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    assert!(f.is_pub);
    assert_eq!(f.export_name.as_deref(), Some("my_custom_add"));
}

#[test]
fn fn_without_export_annotation_has_none() {
    let m = ok("module A\npub fn f(): Int = 1");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    assert_eq!(f.export_name, None);
}

#[test]
fn export_annotation_without_pub_is_error() {
    err("module A\n@export(\"x\")\nfn f(): Int = 1");
}

#[test]
fn export_annotation_before_non_fn_decl_is_error() {
    err("module A\n@export(\"x\")\ntype T = Int");
}

#[test]
fn unknown_annotation_is_error() {
    err("module A\n@bogus(\"x\")\npub fn f(): Int = 1");
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

#[test]
fn priv_constructor_newtype() {
    let m = ok("module A\ntype Email = priv Email(Text)");
    match &m.decls[0].node {
        Decl::Type(t) => {
            assert!(t.is_priv_ctor, "expected is_priv_ctor to be set");
            match &t.body {
                certo_ast::decl::TypeBody::Sum(variants) => {
                    assert_eq!(variants.len(), 1);
                    assert_eq!(variants[0].name.node, "Email");
                    assert_eq!(variants[0].fields.len(), 1);
                }
                other => panic!("expected sum type, got {other:?}"),
            }
        }
        _ => panic!("expected Decl::Type"),
    }
}

#[test]
fn priv_constructor_with_named_field() {
    let m = ok("module A\ntype UserId = priv UserId(value: Int)");
    match &m.decls[0].node {
        Decl::Type(t) => match &t.body {
            certo_ast::decl::TypeBody::Sum(variants) => {
                assert_eq!(variants[0].fields[0].name.as_ref().map(|n| n.node.as_str()), Some("value"));
            }
            other => panic!("expected sum type, got {other:?}"),
        },
        _ => panic!(),
    }
}

#[test]
fn non_priv_type_has_is_priv_ctor_false() {
    let m = ok("module A\ntype Point = { x: Int, y: Int }");
    match &m.decls[0].node {
        Decl::Type(t) => assert!(!t.is_priv_ctor),
        _ => panic!(),
    }
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
// Arrow lambdas: `(x) => body`, `(x, y) => body`, `() => body`
// ------------------------------------------------------------------ //

fn fn_body(m: &certo_ast::module::Module, decl_idx: usize) -> Expr {
    let Decl::Fn(f) = &m.decls[decl_idx].node else { panic!("expected fn") };
    f.body.as_ref().unwrap().node.clone()
}

#[test]
fn single_param_arrow_lambda_parses() {
    // The exact form from the language spec's own examples — was entirely unparseable.
    let m = ok("module A\nfn f(xs: List<Int>): List<Int> = List.map(xs, (x) => x)");
    let Expr::App { args, .. } = fn_body(&m, 0) else { panic!("expected call") };
    let Expr::Lambda { params, .. } = &args[1].value.node else { panic!("expected lambda arg, got {:?}", args[1].value.node) };
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name.node, "x");
    assert!(params[0].ty.is_none());
}

#[test]
fn multi_param_arrow_lambda_parses() {
    let m = ok("module A\nfn f(xs: List<Int>): Int = List.fold(xs, 0, (acc, x) => acc + x)");
    let Expr::App { args, .. } = fn_body(&m, 0) else { panic!("expected call") };
    let Expr::Lambda { params, .. } = &args[2].value.node else { panic!("expected lambda arg") };
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name.node, "acc");
    assert_eq!(params[1].name.node, "x");
}

#[test]
fn typed_arrow_lambda_params_parse() {
    let m = ok("module A\nfn f(): Int = { val add = (a: Int, b: Int) => a + b\n add(1, 2) }");
    let Expr::Block { stmts, .. } = fn_body(&m, 0) else { panic!("expected block") };
    let certo_ast::expr::Stmt::Val { value, .. } = &stmts[0] else { panic!("expected val") };
    let Expr::Lambda { params, .. } = &value.node else { panic!("expected lambda") };
    assert_eq!(params.len(), 2);
    assert!(params[0].ty.is_some());
    assert!(params[1].ty.is_some());
}

#[test]
fn zero_param_arrow_lambda_parses_as_lambda_not_unit() {
    // `()` alone is the Unit literal; `() => body` must still be recognized as a lambda.
    let m = ok("module A\nfn f(): Int = { val thunk = () => 42\n thunk() }");
    let Expr::Block { stmts, .. } = fn_body(&m, 0) else { panic!("expected block") };
    let certo_ast::expr::Stmt::Val { value, .. } = &stmts[0] else { panic!("expected val") };
    let Expr::Lambda { params, .. } = &value.node else { panic!("expected lambda, got {:?}", value.node) };
    assert_eq!(params.len(), 0);
}

#[test]
fn curried_arrow_lambda_parses() {
    let m = ok("module A\nfn f(): Int = { val add = (a) => (b) => a + b\n add(1)(2) }");
    let Expr::Block { stmts, .. } = fn_body(&m, 0) else { panic!("expected block") };
    let certo_ast::expr::Stmt::Val { value, .. } = &stmts[0] else { panic!("expected val") };
    let Expr::Lambda { params, body, .. } = &value.node else { panic!("expected outer lambda") };
    assert_eq!(params.len(), 1);
    assert!(matches!(&body.node, Expr::Lambda { .. }), "expected nested lambda body, got {:?}", body.node);
}

#[test]
fn parenthesized_expr_and_tuple_still_parse_when_not_followed_by_arrow() {
    // Regression guard: ordinary `(expr)` and tuples must not be misread as lambda params.
    let m = ok("module A\nfn f(): Int = (1 + 2)");
    assert!(matches!(fn_body(&m, 0), Expr::BinOp { .. }));

    let m2 = ok("module A\nfn f(): Int = { val (a, b) = (1, 2)\n a + b }");
    let Expr::Block { stmts, .. } = fn_body(&m2, 0) else { panic!("expected block") };
    let certo_ast::expr::Stmt::Val { value, .. } = &stmts[0] else { panic!("expected val") };
    assert!(matches!(&value.node, Expr::Tuple { .. }), "expected tuple, got {:?}", value.node);
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

// ------------------------------------------------------------------ //
// `??` is right-associative: `a ?? b ?? c` == `a ?? (b ?? c)`
// ------------------------------------------------------------------ //

#[test]
fn null_coalesce_chain_is_right_associative() {
    let m = ok("module A\nfn f(): Text = a ?? b ?? c");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    let Expr::BinOp { op: BinOp::NullCoalesce, left, right, .. } = &f.body.as_ref().unwrap().node
        else { panic!("expected a NullCoalesce BinOp") };
    // left must be the bare `a`, not `a ?? b` — that's what right-associativity means here.
    assert!(matches!(&left.node, Expr::Path { .. }), "expected left operand to be bare `a`, got {:?}", left.node);
    // right must itself be the nested `b ?? c`, not the bare `b`.
    let Expr::BinOp { op: BinOp::NullCoalesce, .. } = &right.node
        else { panic!("expected right operand to be the nested `b ?? c`, got {:?}", right.node) };
}

#[test]
fn null_coalesce_single_still_parses() {
    // Regression guard: the two-operand case must still work after the associativity change.
    let m = ok("module A\nfn f(): Text = a ?? b");
    let Decl::Fn(f) = &m.decls[0].node else { panic!("expected fn decl") };
    assert!(matches!(&f.body.as_ref().unwrap().node, Expr::BinOp { op: BinOp::NullCoalesce, .. }));
}

// ------------------------------------------------------------------ //
// Multi-param function *type* annotations: `(A, B) => C`
// ------------------------------------------------------------------ //

#[test]
fn multi_param_fn_type_annotation_has_two_params() {
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn apply(f: (Int, Int) => Int, a: Int, b: Int): Int = f(a, b)");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Fn { params, ret, .. } = &decl.params[0].ty.node else { panic!("expected a function type") };
    assert_eq!(params.len(), 2, "expected 2 params, got {:?}", params);
    assert!(matches!(&params[0].node, TypeExpr::Named { path, .. } if path.segments[0].node == "Int"));
    assert!(matches!(&params[1].node, TypeExpr::Named { path, .. } if path.segments[0].node == "Int"));
    assert!(matches!(&ret.node, TypeExpr::Named { path, .. } if path.segments[0].node == "Int"));
}

#[test]
fn three_param_fn_type_annotation_has_three_params() {
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn apply(f: (Int, Text, Bool) => Unit): Unit = todo()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Fn { params, .. } = &decl.params[0].ty.node else { panic!("expected a function type") };
    assert_eq!(params.len(), 3, "expected 3 params, got {:?}", params);
}

#[test]
fn zero_param_fn_type_annotation_has_no_params() {
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn apply(f: () => Int): Int = f()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Fn { params, .. } = &decl.params[0].ty.node else { panic!("expected a function type") };
    assert!(params.is_empty(), "expected 0 params, got {:?}", params);
}

#[test]
fn single_param_fn_type_annotation_still_parses() {
    // Regression guard: a lone parenthesized param must not become a
    // 1-element-tuple-turned-single-param or otherwise change shape.
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn apply(f: (Int) => Int, a: Int): Int = f(a)");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Fn { params, .. } = &decl.params[0].ty.node else { panic!("expected a function type") };
    assert_eq!(params.len(), 1, "expected 1 param, got {:?}", params);
    assert!(matches!(&params[0].node, TypeExpr::Named { path, .. } if path.segments[0].node == "Int"));
}

#[test]
fn optional_tuple_param_fn_type_not_split() {
    // `(A, B)? => C` is a single Option<(A,B)> param — the `?` binds to the
    // whole tuple, so this must NOT be split into two params.
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn apply(f: (Int, Int)? => Int): Int = todo()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Fn { params, .. } = &decl.params[0].ty.node else { panic!("expected a function type") };
    assert_eq!(params.len(), 1, "expected 1 (optional-tuple) param, got {:?}", params);
    assert!(matches!(&params[0].node, TypeExpr::Option { inner, .. }
        if matches!(&inner.node, TypeExpr::Tuple { elements, .. } if elements.len() == 2)),
        "expected the single param to be Option<(Int, Int)>, got {:?}", params[0].node);
}

#[test]
fn plain_tuple_type_not_followed_by_arrow_still_parses() {
    // Regression guard: a genuine (non-function) tuple type annotation must
    // still parse as a Tuple, not get accidentally affected by the fn-type fix.
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn f(pair: (Int, Int)): Int = 1");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    assert!(matches!(&decl.params[0].ty.node, TypeExpr::Tuple { elements, .. } if elements.len() == 2));
}

// ------------------------------------------------------------------ //
// Decimal(p, s) — BACKLOG item 128
// ------------------------------------------------------------------ //

#[test]
fn decimal_with_precision_scale_parses() {
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn f(x: Decimal(19, 4)): Unit = todo()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::DecimalParam { precision, scale, .. } = &decl.params[0].ty.node
        else { panic!("expected DecimalParam, got {:?}", decl.params[0].ty.node) };
    assert_eq!(*precision, 19);
    assert_eq!(*scale, 4);
}

#[test]
fn bare_decimal_still_parses_as_named() {
    // Regression guard: plain `Decimal` (no parens) must be unaffected —
    // only the `Decimal(` form is special-cased.
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn f(x: Decimal): Unit = todo()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    assert!(matches!(&decl.params[0].ty.node,
        TypeExpr::Named { path, args, .. } if path.segments[0].node == "Decimal" && args.is_empty()));
}

#[test]
fn decimal_optional_parses() {
    // `Decimal(19, 4)?` — the `?` suffix loop in parse_type must apply on
    // top of the DecimalParam atom just like it does for any other type.
    use certo_ast::types::TypeExpr;
    let m = ok("module A\nfn f(x: Decimal(19, 4)?): Unit = todo()");
    let Decl::Fn(decl) = &m.decls[0].node else { panic!("expected fn decl") };
    let TypeExpr::Option { inner, .. } = &decl.params[0].ty.node
        else { panic!("expected Option, got {:?}", decl.params[0].ty.node) };
    assert!(matches!(&inner.node, TypeExpr::DecimalParam { .. }));
}

#[test]
fn decimal_missing_comma_is_error() {
    err("module A\nfn f(x: Decimal(19 4)): Unit = todo()");
}

#[test]
fn decimal_non_integer_arg_is_error() {
    err("module A\nfn f(x: Decimal(p, s)): Unit = todo()");
}

#[test]
fn decimal_out_of_range_precision_is_error() {
    // Precision/scale are u8 (0-255) — 999 doesn't fit.
    err("module A\nfn f(x: Decimal(999, 4)): Unit = todo()");
}

// ------------------------------------------------------------------ //
// `live val` inside a view (BACKLOG item 88 — parsing slice only)
// ------------------------------------------------------------------ //

fn parse_view_decl(src: &str) -> certo_ast::decl::ViewDecl {
    let m = ok(src);
    m.decls.iter().find_map(|d| match &d.node {
        Decl::View(v) => Some(v.clone()),
        _ => None,
    }).expect("expected a view decl")
}

#[test]
fn live_val_parses_into_view_live_field() {
    let v = parse_view_decl(
        "module A\nview D {\n live val count = 0\n layout = Heading(\"x\")\n}"
    );
    assert_eq!(v.live.len(), 1);
    let certo_ast::pattern::Pattern::Ident { name, .. } = &v.live[0].pattern.node else {
        panic!("expected Ident pattern, got {:?}", v.live[0].pattern.node);
    };
    assert_eq!(name.node, "count");
}

#[test]
fn multiple_live_vals_all_parsed() {
    let v = parse_view_decl(
        "module A\nview D {\n live val a = 1\n live val b = \"x\"\n layout = Heading(\"x\")\n}"
    );
    assert_eq!(v.live.len(), 2);
}

#[test]
fn view_without_live_val_has_empty_live_field() {
    // No regression — a view with no `live val` still parses, with an
    // empty `live` list (not an error, not a leftover from another view).
    let v = parse_view_decl("module A\nview D {\n layout = Heading(\"x\")\n}");
    assert!(v.live.is_empty());
}

#[test]
fn live_as_a_plain_identifier_elsewhere_is_unaffected() {
    // `live` is a contextual/soft keyword recognized only inside a view
    // body immediately before `val` (same pattern as `pk`/`filter`/
    // `layout`) — using it as an ordinary function or variable name
    // anywhere else must keep working exactly as before.
    ok("module A\nfn live(): Int = 42\nval live: Int = 7");
}

#[test]
fn live_val_reuses_ordinary_val_syntax_including_type_annotation() {
    let v = parse_view_decl(
        "module A\nview D {\n live val n: Int = 0\n layout = Heading(\"x\")\n}"
    );
    assert_eq!(v.live.len(), 1);
    assert!(v.live[0].ty.is_some());
}

// ------------------------------------------------------------------ //
// `@ui.generate` (BACKLOG item 87 — parsing only, lowering is in crates/ui)
// ------------------------------------------------------------------ //

fn parse_ui_generate_decl(src: &str) -> certo_ast::decl::UiGenerateDecl {
    let m = ok(src);
    m.decls.iter().find_map(|d| match &d.node {
        Decl::UiGenerate(g) => Some(g.clone()),
        _ => None,
    }).expect("expected a @ui.generate decl")
}

#[test]
fn ui_generate_parses_type_name_title_and_columns() {
    let g = parse_ui_generate_decl(
        "module A\n@ui.generate(Product) {\n title: \"Products\"\n list: { columns: [name, sku, price] }\n}"
    );
    assert_eq!(g.type_name.node, "Product");
    assert_eq!(g.title.as_deref(), Some("Products"));
    assert_eq!(g.columns, vec!["name", "sku", "price"]);
}

#[test]
fn ui_generate_with_empty_block_has_no_title_or_columns() {
    let g = parse_ui_generate_decl("module A\n@ui.generate(Product) {}");
    assert_eq!(g.type_name.node, "Product");
    assert_eq!(g.title, None);
    assert!(g.columns.is_empty());
}

#[test]
fn ui_generate_rejects_unsupported_list_key() {
    err("module A\n@ui.generate(Product) {\n list: { sortable: [name] }\n}");
}

#[test]
fn ui_generate_rejects_unsupported_top_level_key() {
    err("module A\n@ui.generate(Product) {\n permissions: { view: [Admin] }\n}");
}

#[test]
fn export_annotation_still_parses_after_ui_generate_lookahead() {
    // The new `@ui.generate` lookahead in `parse_decl` must not disturb
    // the pre-existing `@export("name")` path for anything that isn't
    // `@ui.generate` specifically.
    ok("module A\n@export(\"add\")\npub fn add(a: Int, b: Int): Int = a + b");
}

#[test]
fn unknown_at_annotation_still_errors_as_before() {
    err("module A\n@bogus(\"x\")\npub fn f(): Int = 1");
}

// ------------------------------------------------------------------ //
// `every(interval) { body }` — BACKLOG item 122/141
// ------------------------------------------------------------------ //

fn parse_fn_body(src: &str) -> S<Expr> {
    let m = ok(src);
    m.decls.iter().find_map(|d| match &d.node {
        Decl::Fn(f) => f.body.clone(),
        _ => None,
    }).expect("expected an fn decl with a body")
}

#[test]
fn every_desugars_to_spawn_of_while_true() {
    let body = parse_fn_body(
        "module A\nasync fn f(): Unit = every(Duration.seconds(1)) {\n println(\"tick\")\n}");
    let Expr::Spawn { expr, .. } = &body.node else { panic!("expected Expr::Spawn, got {:?}", body.node) };
    let Expr::While { cond, body: loop_body, .. } = &expr.node else { panic!("expected Expr::While, got {:?}", expr.node) };
    assert!(matches!(&cond.node, Expr::Lit { value: Lit::Bool(true), .. }), "expected `while true`, got {:?}", cond.node);
    let Expr::Block { stmts, .. } = &loop_body.node else { panic!("expected a block body") };
    // First statement is the synthesized `sleep(Duration.toSeconds(interval) * 1000)`;
    // the user's own `println("tick")` follows it.
    assert_eq!(stmts.len(), 2, "expected [sleep(...), println(...)], got {:?}", stmts);
}

#[test]
fn every_as_plain_identifier_elsewhere_is_unaffected() {
    // `every` is a contextual identifier — only special when directly
    // followed by `(`, matching the soft-keyword pattern already
    // established for `live`/`unsafe`/`pk`/`filter`/`layout`.
    ok("module A\nfn f(every: Int): Int = every + 1");
    ok("module A\nfn f(): Int = {\n val every = 5\n every\n}");
}
