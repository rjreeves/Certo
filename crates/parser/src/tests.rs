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
