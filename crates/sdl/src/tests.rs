use crate::*;

fn ok(src: &str) -> SdlFile {
    let (f, d) = parse(src);
    assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
    f
}

fn table(f: &SdlFile, i: usize) -> &TableDecl {
    match &f.decls[i] { Decl::Table(t) => t, o => panic!("not a table: {o:?}") }
}

#[test]
fn table_with_columns_and_mods() {
    let f = ok("table users {\n id: uuid primary key\n name: text not null unique\n bio: text null\n age: int default 18\n}");
    let t = table(&f, 0);
    assert_eq!(t.name.name, "users");
    assert_eq!(t.members.len(), 4);
    let Member::Column(id) = &t.members[0] else { panic!() };
    assert!(matches!(id.mods[0], ColumnMod::PrimaryKey(_)));
    let Member::Column(name) = &t.members[1] else { panic!() };
    assert!(matches!(name.mods[..], [ColumnMod::NotNull(_), ColumnMod::Unique(_)]));
    let Member::Column(age) = &t.members[3] else { panic!() };
    assert!(matches!(&age.mods[0], ColumnMod::Default(Expr::Literal(Literal::Number(18), _))));
}

#[test]
fn relationships() {
    let f = ok("table posts { author: users -> one  editor: users -> optional  tags: tag -> many }");
    let t = table(&f, 0);
    let cards: Vec<_> = t.members.iter().map(|m| match m {
        Member::Rel(r) => r.cardinality, _ => panic!(),
    }).collect();
    assert_eq!(cards, [Cardinality::One, Cardinality::Optional, Cardinality::Many]);
}

#[test]
fn contextual_keywords_are_valid_names() {
    // columns named after modifier / cardinality words
    let f = ok("table t { key: text unique: text null: text many: text }");
    assert_eq!(table(&f, 0).members.len(), 4);
}

#[test]
fn enum_type_index_constraint() {
    let f = ok("enum Role { admin, user }\n\
                type Address { street: text city: text }\n\
                index idx on users (name, email)\n\
                constraint adult on users using age >= 18 and active == true");
    assert_eq!(f.decls.len(), 4);
    let Decl::Enum(e) = &f.decls[0] else { panic!() };
    assert_eq!(e.variants.len(), 2);
    let Decl::Type(t) = &f.decls[1] else { panic!() };
    assert_eq!(t.fields.len(), 2);
    let Decl::Index(i) = &f.decls[2] else { panic!() };
    assert_eq!(i.columns.len(), 2);
    let Decl::Constraint(c) = &f.decls[3] else { panic!() };
    assert!(matches!(c.expr, Expr::Binary { op: BinaryOp::And, .. }));
}

#[test]
fn precedence_and_associativity() {
    let f = ok("constraint c on t using 1 + 2 * 3 - 4");
    let Decl::Constraint(c) = &f.decls[0] else { panic!() };
    // ((1 + (2 * 3)) - 4)
    let Expr::Binary { op: BinaryOp::Sub, lhs, .. } = &c.expr else { panic!("{:?}", c.expr) };
    let Expr::Binary { op: BinaryOp::Add, rhs, .. } = &**lhs else { panic!() };
    assert!(matches!(**rhs, Expr::Binary { op: BinaryOp::Mul, .. }));

    let f = ok("constraint c on t using a or b and c");
    let Decl::Constraint(c) = &f.decls[0] else { panic!() };
    let Expr::Binary { op: BinaryOp::Or, rhs, .. } = &c.expr else { panic!() };
    assert!(matches!(**rhs, Expr::Binary { op: BinaryOp::And, .. }));
}

#[test]
fn calls_parens_strings_comments() {
    let f = ok("// header\ntable t { at: timestamp default now()\n s: text default \"a\\\"b\" // trailing\n n: int default (1 + 2) }");
    let t = table(&f, 0);
    let Member::Column(at) = &t.members[0] else { panic!() };
    assert!(matches!(&at.mods[0], ColumnMod::Default(Expr::Call { args, .. }) if args.is_empty()));
    let Member::Column(s) = &t.members[1] else { panic!() };
    assert!(matches!(&s.mods[0], ColumnMod::Default(Expr::Literal(Literal::String(v), _)) if v == "a\"b"));
}

#[test]
fn default_followed_by_next_column() {
    let f = ok("table t { a: int default 1  b: int default x and y  and: int }");
    assert_eq!(table(&f, 0).members.len(), 3);
}

#[test]
fn errors_recover_at_next_declaration() {
    let (f, d) = parse("table broken { a int }\nenum Ok { x }\ntable also_bad {\nenum Fine { y }");
    assert!(d.len() >= 2, "{d:?}");
    let kinds: Vec<_> = f.decls.iter().map(|d| matches!(d, Decl::Enum(_))).collect();
    assert_eq!(kinds, [true, true]);
}

#[test]
fn lexical_errors() {
    let (_, d) = parse("table t { a: int default \"open }");
    assert!(d.iter().any(|x| x.code == "SDL003"));
    let (_, d) = parse("table t { a: int default 99999999999999999999999 }");
    assert!(d.iter().any(|x| x.code == "SDL002"));
    let (_, d) = parse("table t { a: int } @");
    assert!(d.iter().any(|x| x.code == "SDL001"));
}

#[test]
fn spans_point_at_source() {
    let src = "table users { id: uuid }";
    let f = ok(src);
    let t = table(&f, 0);
    assert_eq!(&src[t.name.span.start as usize..t.name.span.end as usize], "users");
    assert_eq!(&src[t.span.start as usize..t.span.end as usize], src);
}

#[test]
fn oversized_expressions_are_rejected_not_crashed() {
    let chain = vec!["1"; 100_000].join(" + ");
    let (_, d) = parse(&format!("constraint c on t using {chain}"));
    assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    let nested = format!("{}1{}", "(".repeat(50_000), ")".repeat(50_000));
    let (_, d) = parse(&format!("constraint c on t using {nested}"));
    assert!(d.iter().any(|x| x.code == "SDL101"));
    // ordinary large-ish expressions still fine
    let ok = vec!["1"; 100].join(" + ");
    let (_, d) = parse(&format!("constraint c on t using {ok}"));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn new_tokens_lex_but_are_not_sdl() {
    let (toks, d) = lex("a.b = c == d");
    assert!(d.is_empty());
    let kinds: Vec<_> = toks.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(kinds[1], TokKind::Dot);
    assert_eq!(kinds[3], TokKind::Eq);
    assert_eq!(kinds[5], TokKind::EqEq);
    // in an SDL file they are ordinary syntax errors
    let (_, d) = parse("table t { a. }");
    assert!(d.iter().any(|x| x.code == "SDL100"));
}

#[test]
fn parser_api_is_usable_by_sibling_languages() {
    let mut p = Parser::new("x + 2 rest");
    let e = p.expr().unwrap();
    assert!(matches!(e, Expr::Binary { op: BinaryOp::Add, .. }));
    assert!(p.at_word("rest"));
    assert!(!p.at_eof());
    p.recover_to(&["nothing"]);
    assert!(p.at_eof());
    assert!(p.into_diagnostics().is_empty());
}

#[test]
fn the_deepest_allowed_expression_compiles_on_a_small_stack() {
    // 127 terms = 126 operators + 1: right at the depth limit (128)
    let chain = vec!["1"; 126].join(" + ");
    let src = format!("table t {{ id: int primary key }} constraint c on t using {chain} > 0");
    let handle = std::thread::Builder::new().stack_size(1024 * 1024).spawn(move || compile(&src)).unwrap();
    let (ir, d) = handle.join().expect("no stack overflow on a 1 MB stack");
    assert!(ir.is_some(), "{d:?}");

    // one level deeper is refused with a clear message, not a crash
    for src in [
        format!("constraint c on t using {}", vec!["1"; 200].join(" + ")),
        format!("constraint c on t using {}true", "not ".repeat(300)),
        format!("constraint c on t using {}1{}", "(".repeat(200), ")".repeat(200)),
    ] {
        let (_, d) = parse(&src);
        assert!(d.iter().any(|x| x.code == "SDL101"), "{d:?}");
    }

    // long FLAT lists are fine: they add elements, not depth
    let list = vec!["1"; 150].join(", ");
    let (ir, d) = compile(&format!("table t {{ id: int primary key }} constraint c on t using id in ({list})"));
    assert!(ir.is_some(), "{d:?}");
}

#[test]
fn a_long_run_of_not_is_refused_before_it_can_overflow_the_stack() {
    // 450 stays under the 500-element limit, so only the nesting guard stops it
    let nots = format!("{}true", "not ".repeat(450));
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || parse(&format!("constraint c on t using {nots}")))
        .unwrap();
    let (_, d) = handle.join().expect("no stack overflow on a 1 MB stack");
    assert!(d.iter().any(|x| x.code == "SDL101" && x.message.contains("nested")), "{d:?}");
}
