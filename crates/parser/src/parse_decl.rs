use std::collections::HashSet;
use certo_ast::span::{S, Span};
use certo_ast::decl::*;
use certo_ast::expr::{Expr, Stmt, Lit, FStringPart, ExpectMatcher};
use certo_ast::pattern::Pattern;
use certo_ast::types::{TypeExpr, ModulePath, TypeParam};
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};
use crate::parse_type::{parse_type, parse_type_params, parse_module_path, parse_effect_set};
use crate::parse_expr::{parse_expr, parse_block};
use crate::parse_pattern::parse_pattern;

/// Returns more than one declaration only for `extern` blocks (handled
/// separately in `parse_module.rs`, before this function is ever called)
/// and for a `type` declaration whose record body contains in-body `fn`
/// methods (BACKLOG item 150) — desugared here into the original `TypeDecl`
/// plus a synthesized `ImplDecl` carrying those methods, so every later
/// compiler stage (resolve/typeck/hir/mir/codegen) sees exactly the same
/// AST shape a hand-written `impl X { ... }` block already produces, with
/// zero changes needed anywhere downstream of the parser.
pub fn parse_decl(cur: &mut Cursor<'_>) -> Result<Vec<S<Decl>>, ParseError> {
    let span = cur.peek_span();
    if let Some(d) = try_parse_ui_generate(cur)? {
        let s = d.span;
        return Ok(vec![S::new(Decl::UiGenerate(d), s)]);
    }
    let type_annotations = parse_type_marker_annotations(cur)?;
    let export_name = parse_export_annotation(cur)?;
    let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();

    // Checked before the match below (rather than as a match guard) since
    // `Some(Token::Fn)`'s own arm has no guard and would otherwise
    // unconditionally win over a guarded catch-all for the same pattern,
    // silently discarding a `@valueObject`/`@aggregate` written before a
    // `fn` instead of rejecting it.
    if !type_annotations.is_empty() && cur.peek() != Some(&Token::Type) {
        return Err(ParseError {
            kind: ParseErrorKind::Custom("`@valueObject`/`@aggregate` may only be used before `type`".into()),
            span,
        });
    }

    match cur.peek() {
        Some(Token::Async) | Some(Token::Fn) => {
            let mut d = parse_fn_decl(cur, is_pub)?;
            if let Some(name) = export_name {
                if !is_pub {
                    return Err(ParseError {
                        kind: ParseErrorKind::Custom("`@export(...)` requires `pub fn`".into()),
                        span,
                    });
                }
                d.export_name = Some(name);
            }
            let s = d.span;
            Ok(vec![S::new(Decl::Fn(d), s)])
        }
        _ if export_name.is_some() => Err(ParseError {
            kind: ParseErrorKind::Custom("`@export(...)` may only be used before `pub fn`".into()),
            span,
        }),
        Some(Token::Type) => {
            let (d, methods) = parse_type_decl(cur, is_pub, type_annotations)?;
            let s = d.span;
            let mut decls = vec![S::new(Decl::Type(d.clone()), s)];
            if !methods.is_empty() {
                let impl_decl = ImplDecl {
                    trait_path: None,
                    type_path: certo_ast::types::ModulePath {
                        segments: vec![d.name.clone()],
                        span: d.name.span,
                    },
                    type_params: d.type_params.clone(),
                    methods,
                    span: s,
                };
                decls.push(S::new(Decl::Impl(impl_decl), s));
            }
            Ok(decls)
        }
        Some(Token::Val) => {
            let d = parse_val_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Val(d), s)])
        }
        Some(Token::Var) => {
            let d = parse_var_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Var(d), s)])
        }
        Some(Token::Trait) => {
            let d = parse_trait_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Trait(d), s)])
        }
        Some(Token::Impl) => {
            let d = parse_impl_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Impl(d), s)])
        }
        Some(Token::StateMachine) => {
            let d = parse_statemachine(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::StateMachine(d), s)])
        }
        Some(Token::Migration) => {
            let d = parse_migration(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Migration(d), s)])
        }
        Some(Token::View) => {
            let d = parse_view(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::View(d), s)])
        }
        Some(Token::Form) => {
            let d = parse_form(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Form(d), s)])
        }
        Some(Token::Test) => {
            let d = parse_test_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Test(d), s)])
        }
        Some(Token::Property) => {
            let d = parse_property_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Property(d), s)])
        }
        Some(Token::DbTest) => {
            let d = parse_dbtest_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::DbTest(d), s)])
        }
        Some(Token::Import) => {
            let d = parse_import_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Import(d), s)])
        }
        Some(Token::Validator) => {
            let d = parse_validator_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Validator(d), s)])
        }
        Some(Token::Constraint) => {
            let d = parse_constraint_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Constraint(d), s)])
        }
        Some(Token::Temporal) => {
            let d = parse_temporal_decl(cur, is_pub)?;
            let s = d.span;
            Ok(vec![S::new(Decl::Temporal(d), s)])
        }
        Some(Token::RuleTest) => {
            let d = parse_rule_test_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::RuleTest(d), s)])
        }
        Some(Token::ValidatorTest) => {
            let d = parse_validator_test_decl(cur)?;
            let s = d.span;
            Ok(vec![S::new(Decl::ValidatorTest(d), s)])
        }
        other => {
            let found = other.map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
            Err(ParseError {
                kind: ParseErrorKind::Expected { expected: "declaration".into(), found },
                span,
            })
        }
    }
}

/// `@export("cSymbolName")` — optional annotation immediately before a
/// `pub fn`, overriding the generated C export name (default `certo_<name>`)
/// for FFI interop. No general `@`-annotation system exists yet — this is
/// parsed narrowly for just this one form, matching the spec's only
/// documented usage (`docs/Certo_Language_Specification.md` §12.3).
/// `@ui.generate(TypeName) { title: "...", list: { columns: [...] } }` —
/// BACKLOG item 87. Deliberately a standalone `@`-form recognized by exact
/// dotted name (`@` `ui` `.` `generate`), not a general annotation system —
/// `@export("name")` (`parse_export_annotation` below) is the only other
/// `@`-form, and it's just as narrowly hardcoded to its own exact shape.
/// Returns `Ok(None)` without consuming anything if the tokens don't match
/// (so `@export(...)` still parses normally afterward); only commits once
/// `@ui.generate` is confirmed present.
fn try_parse_ui_generate(cur: &mut Cursor<'_>) -> Result<Option<UiGenerateDecl>, ParseError> {
    let is_ui_generate = matches!(cur.peek(), Some(Token::At))
        && matches!(cur.peek2(), Some(Token::Ident(s)) if *s == "ui")
        && matches!(cur.peek3(), Some(Token::Dot))
        && matches!(cur.peek4(), Some(Token::Ident(s)) if *s == "generate");
    if !is_ui_generate {
        return Ok(None);
    }
    let (_, at_span) = cur.bump().unwrap(); // @
    cur.bump(); // ui
    cur.bump(); // .
    cur.bump(); // generate

    cur.expect(&Token::LParen)?;
    let (type_name, type_name_span) = cur.expect_ident()?;
    cur.expect(&Token::RParen)?;

    let mut title: Option<String> = None;
    let mut columns: Vec<String> = Vec::new();

    cur.expect(&Token::LBrace)?;
    loop {
        if cur.peek() == Some(&Token::RBrace) { break; }
        let (key, key_span) = cur.expect_ident()?;
        match key.as_str() {
            "title" => {
                cur.expect(&Token::Colon)?;
                let (tok, tok_span) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: key_span })?;
                let Token::StringLit(s) = tok else {
                    return Err(ParseError {
                        kind: ParseErrorKind::Expected { expected: "string literal".into(), found: format!("{tok:?}") },
                        span: tok_span,
                    });
                };
                title = Some(s.to_string());
            }
            "list" => {
                cur.expect(&Token::Colon)?;
                cur.expect(&Token::LBrace)?;
                loop {
                    if cur.peek() == Some(&Token::RBrace) { break; }
                    let (subkey, subkey_span) = cur.expect_ident()?;
                    match subkey.as_str() {
                        "columns" => {
                            cur.expect(&Token::Colon)?;
                            cur.expect(&Token::LBracket)?;
                            loop {
                                if cur.peek() == Some(&Token::RBracket) { break; }
                                let (col, _) = cur.expect_ident()?;
                                columns.push(col);
                                cur.eat(|t| matches!(t, Token::Comma));
                            }
                            cur.expect(&Token::RBracket)?;
                        }
                        // `sortable`/`filterable`/`searchable` are real spec
                        // fields, deliberately not accepted here — each needs
                        // codegen capability that doesn't exist yet
                        // (BACKLOG item 87), so rejecting them clearly beats
                        // silently accepting and ignoring them.
                        other => return Err(ParseError {
                            kind: ParseErrorKind::Custom(format!(
                                "`@ui.generate`'s `list` block does not support `{other}` yet — only `columns` is implemented"
                            )),
                            span: subkey_span,
                        }),
                    }
                    cur.eat(|t| matches!(t, Token::Comma));
                }
                cur.expect(&Token::RBrace)?;
            }
            // `detail`/`form`/`permissions` are real spec fields, same
            // reasoning as `sortable`/etc. above — not implemented, and
            // rejected rather than silently ignored.
            other => return Err(ParseError {
                kind: ParseErrorKind::Custom(format!(
                    "`@ui.generate` does not support `{other}` yet — only `title` and `list.columns` are implemented"
                )),
                span: key_span,
            }),
        }
        cur.eat(|t| matches!(t, Token::Comma));
    }
    let end = cur.expect(&Token::RBrace)?;

    Ok(Some(UiGenerateDecl {
        type_name: S::new(type_name, type_name_span),
        title,
        columns,
        span: at_span.to(end),
    }))
}

/// `@valueObject` / `@aggregate` (spec §8.4/§8.5) — zero or more bare marker
/// annotations before a `type` declaration (BACKLOG item 149). Recognized by
/// exact name only, the same narrow-hardcoded-shape precedent `@export`/
/// `@ui.generate` already establish rather than a general `@`-decorator
/// system. Does not consume anything (returns an empty `Vec`) if the next
/// token isn't one of these two exact names, so `@export(...)`/`@ui.generate`
/// still parse normally afterward.
fn parse_type_marker_annotations(cur: &mut Cursor<'_>) -> Result<Vec<String>, ParseError> {
    let mut annotations = Vec::new();
    loop {
        let is_marker = matches!(cur.peek(), Some(Token::At))
            && matches!(cur.peek2(), Some(Token::Ident(s)) if *s == "valueObject" || *s == "aggregate");
        if !is_marker {
            break;
        }
        cur.bump(); // @
        let (name, _) = cur.expect_ident()?;
        annotations.push(name);
    }
    Ok(annotations)
}

fn parse_export_annotation(cur: &mut Cursor<'_>) -> Result<Option<String>, ParseError> {
    if cur.eat(|t| matches!(t, Token::At)).is_none() {
        return Ok(None);
    }
    let word_span = cur.peek_span();
    // `export` is a reserved keyword (`Token::Export`), not a plain identifier.
    if cur.eat(|t| matches!(t, Token::Export)).is_none() {
        let found = cur.peek().map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
        return Err(ParseError {
            kind: ParseErrorKind::Custom(format!("unknown annotation `@{found}` — only `@export(\"name\")` is supported")),
            span: word_span,
        });
    }
    cur.expect(&Token::LParen)?;
    let (tok, tok_span) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: word_span })?;
    let name = if let Token::StringLit(s) = tok {
        s.to_string()
    } else {
        return Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "string literal".into(), found: format!("{tok:?}") },
            span: tok_span,
        });
    };
    cur.expect(&Token::RParen)?;
    Ok(Some(name))
}

// ------------------------------------------------------------------ //
// Function
// ------------------------------------------------------------------ //

pub fn parse_fn_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<FnDecl, ParseError> {
    let start = cur.peek_span();
    let is_async = cur.eat(|t| matches!(t, Token::Async)).is_some();
    cur.expect(&Token::Fn)?;

    let (name, name_span) = cur.expect_ident()?;
    let type_params = parse_type_params(cur)?;

    cur.expect(&Token::LParen)?;
    let mut params = Vec::new();
    while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
        params.push(parse_fn_param(cur)?);
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }
    cur.expect(&Token::RParen)?;

    let ret_ty = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
        Some(parse_type(cur)?)
    } else {
        None
    };

    let effects = parse_effect_set(cur)?;

    let body = if cur.eat(|t| matches!(t, Token::Eq)).is_some() {
        Some(parse_expr(cur)?)
    } else {
        None
    };

    let span = start.to(body.as_ref().map(|b| b.span).unwrap_or(name_span));
    Ok(FnDecl { is_async, is_pub, name: S::new(name, name_span), type_params, params, ret_ty, effects, body, is_extern: false, export_name: None, span })
}

/// Parse an `extern "C" { fn name(params): Ret ... }` block into a list of
/// body-less `FnDecl`s flagged `is_extern`. Each signature is hoisted like a
/// normal function so calls type-check; codegen emits a prototype and the
/// definition is resolved from a native library linked with `--link`.
pub fn parse_extern_block(cur: &mut Cursor<'_>) -> Result<Vec<S<Decl>>, ParseError> {
    cur.expect(&Token::Extern)?;
    // Optional ABI string (e.g. "C"). Only the C ABI is supported; accept and ignore.
    cur.eat(|t| matches!(t, Token::StringLit(_)));
    cur.expect(&Token::LBrace)?;
    let mut decls = Vec::new();
    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let mut fd = parse_fn_decl(cur, false)?;
        fd.is_extern = true;
        let s = fd.span;
        decls.push(S::new(Decl::Fn(fd), s));
    }
    cur.expect(&Token::RBrace)?;
    Ok(decls)
}

fn parse_fn_param(cur: &mut Cursor<'_>) -> Result<FnParam, ParseError> {
    let (name, name_span) = cur.expect_ident()?;
    cur.expect(&Token::Colon)?;
    let ty = parse_type(cur)?;
    let default = if cur.eat(|t| matches!(t, Token::Eq)).is_some() {
        Some(parse_expr(cur)?)
    } else {
        None
    };
    let span = name_span.to(ty.span);
    Ok(FnParam { name: S::new(name, name_span), ty, default, span })
}

// ------------------------------------------------------------------ //
// Type declaration
// ------------------------------------------------------------------ //

/// Returns the parsed `TypeDecl` plus any in-body `fn` methods found inside
/// a record body (BACKLOG item 150) — empty for every type shape other than
/// `TypeBody::Record`, and for a record body with no in-body methods.
/// `parse_decl`'s own `Token::Type` arm is what actually desugars a
/// non-empty methods list into a synthesized `ImplDecl`; this function just
/// threads them up from `parse_record_type_def`.
fn parse_type_decl(cur: &mut Cursor<'_>, is_pub: bool, annotations: Vec<String>) -> Result<(TypeDecl, Vec<FnDecl>), ParseError> {
    let start = cur.expect(&Token::Type)?;
    let (name, name_span) = cur.expect_ident()?;
    let type_params = parse_type_params(cur)?;
    cur.expect(&Token::Eq)?;

    // `type X = priv X(...)` — smart-constructor newtype: a single sum
    // variant sharing the type's own name, whose raw constructor call is
    // restricted to `impl X { ... }` blocks (enforced in typeck).
    let is_priv_ctor = cur.eat(|t| matches!(t, Token::Priv)).is_some();

    let mut methods = Vec::new();
    let body = if is_priv_ctor {
        TypeBody::Sum(parse_sum_variants_no_leading_bar(cur)?)
    } else if cur.peek() == Some(&Token::LBrace) {
        let (rec, ms) = parse_record_type_def(cur)?;
        methods = ms;
        // `computed name: Ty = expr` (BACKLOG item 143) — desugared here
        // into a real method on the same synthesized `impl` block item
        // 150's in-body `fn`s already use, so field-access resolution
        // (`crates/hir/src/lower.rs`'s `Expr::Field` arm) only needs to
        // emit an ordinary call, not a new evaluation path. Each synthesized
        // method's body prepends `val field = self.field` for every real
        // stored field before the original (unmodified) computed
        // expression — the same "destructure into locals" trick
        // `let { x, y } = point` already uses — so a bare `paidAt` inside
        // `computed isPaid: Bool = paidAt.isSome()` resolves as an
        // ordinary local, with zero changes to how expression bodies are
        // lowered anywhere downstream.
        for c in &rec.computed {
            methods.push(synthesize_computed_method(&name, name_span, &type_params, &rec.fields, &rec.computed, c));
        }
        TypeBody::Record(rec)
    } else if cur.peek() == Some(&Token::Bar) {
        // `type T = | Variant1 | Variant2` — leading bar present
        TypeBody::Sum(parse_sum_variants(cur)?)
    } else {
        // Peek ahead: if this looks like `Ident | ...` it's a sum type without leading bar.
        // Detect: first token is uppercase Ident and second is Bar.
        let is_sum = matches!(cur.peek(), Some(Token::Ident(s)) if s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
            && cur.peek2() == Some(&Token::Bar);
        if is_sum {
            TypeBody::Sum(parse_sum_variants_no_leading_bar(cur)?)
        } else {
            TypeBody::Alias(parse_type(cur)?)
        }
    };

    let span = start.to(cur.peek_span());
    Ok((TypeDecl { is_pub, is_priv_ctor, annotations, name: S::new(name, name_span), type_params, body, span }, methods))
}

/// Collects every bare (single-segment) identifier referenced anywhere in
/// an expression tree (BACKLOG item 173) — used to decide which *other*
/// `computed` properties on the same record a synthesized computed
/// accessor's prelude should also bind. Deliberately approximate: it does
/// not distinguish a genuinely free reference from one already shadowed by
/// an inner `val`/lambda-param/match-binding of the same name (a bare
/// identifier is collected either way) — an extra, unused prelude binding
/// is harmless, the same tradeoff the unconditional stored-field prelude
/// below already accepts, so a real scope-tracking pass isn't needed here.
fn collect_referenced_idents(expr: &S<Expr>, out: &mut HashSet<String>) {
    match &expr.node {
        Expr::Lit { value, .. } => {
            if let Lit::FString(parts) = value {
                for p in parts {
                    if let FStringPart::Interpolated(e) = p { collect_referenced_idents(e, out); }
                }
            }
        }
        Expr::Path { path, .. } => {
            if path.segments.len() == 1 { out.insert(path.segments[0].node.clone()); }
        }
        Expr::App { func, args, .. } => {
            collect_referenced_idents(func, out);
            for a in args { collect_referenced_idents(&a.value, out); }
        }
        Expr::Pipe { left, right, .. } | Expr::BinOp { left, right, .. } => {
            collect_referenced_idents(left, out);
            collect_referenced_idents(right, out);
        }
        Expr::UnOp { expr, .. }
        | Expr::Field { expr, .. }
        | Expr::SafeField { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Ascribe { expr, .. }
        | Expr::Age { expr, .. } => collect_referenced_idents(expr, out),
        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => collect_referenced_idents(body, out),
        Expr::If { cond, then_expr, else_expr, .. } => {
            collect_referenced_idents(cond, out);
            collect_referenced_idents(then_expr, out);
            collect_referenced_idents(else_expr, out);
        }
        Expr::Match { scrutinee, arms, .. } => {
            collect_referenced_idents(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard { collect_referenced_idents(g, out); }
                collect_referenced_idents(&arm.body, out);
            }
        }
        Expr::Block { stmts, .. } => {
            for s in stmts {
                match s {
                    Stmt::Val { value, .. } => collect_referenced_idents(value, out),
                    Stmt::Var { value, .. } => collect_referenced_idents(value, out),
                    Stmt::Assign { value, .. } => collect_referenced_idents(value, out),
                    Stmt::Defer { body, .. } => collect_referenced_idents(body, out),
                    Stmt::Expr { expr, .. } => collect_referenced_idents(expr, out),
                }
            }
        }
        Expr::Lambda { body, .. } => collect_referenced_idents(body, out),
        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { collect_referenced_idents(e, out); }
        }
        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { collect_referenced_idents(b, out); }
            for f in fields { collect_referenced_idents(&f.value, out); }
        }
        Expr::Guard { cond, else_expr, .. } => {
            collect_referenced_idents(cond, out);
            collect_referenced_idents(else_expr, out);
        }
        Expr::Require { expr, error, .. } => {
            collect_referenced_idents(expr, out);
            collect_referenced_idents(error, out);
        }
        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { collect_referenced_idents(t, out); }
            if let Some(t) = timeout { collect_referenced_idents(t, out); }
        }
        Expr::For { iter, body, .. } => {
            collect_referenced_idents(iter, out);
            collect_referenced_idents(body, out);
        }
        Expr::While { cond, body, .. } => {
            collect_referenced_idents(cond, out);
            collect_referenced_idents(body, out);
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            collect_referenced_idents(actual, out);
            if let ExpectMatcher::ToBe(e) = matcher { collect_referenced_idents(e, out); }
        }
    }
}

/// Desugars one `computed name: Ty = body` into a real method
/// `fn name(self: TypeName<...>): Ty = { val f1 = self.f1; ...; body }`
/// (BACKLOG item 143). `self`'s own type carries the record's type params
/// as bare references (`TypeExpr::Param`), matching how a hand-written
/// generic method's own receiver parameter would be written.
fn synthesize_computed_method(
    type_name: &str,
    type_name_span: Span,
    type_params: &[TypeParam],
    fields: &[RecordFieldDef],
    all_computed: &[ComputedFieldDef],
    c: &ComputedFieldDef,
) -> FnDecl {
    let self_ty_args: Vec<S<TypeExpr>> = type_params.iter()
        .map(|tp| S::new(TypeExpr::Param { name: tp.name.clone(), span: tp.name.span }, tp.name.span))
        .collect();
    let self_ty = S::new(TypeExpr::Named {
        path: ModulePath { segments: vec![S::new(type_name.to_string(), type_name_span)], span: type_name_span },
        args: self_ty_args,
        span: type_name_span,
    }, type_name_span);
    let self_name = S::new("self".to_string(), c.span);
    let self_param = FnParam { name: self_name.clone(), ty: self_ty, default: None, span: c.span };

    let mut stmts: Vec<Stmt> = fields.iter().map(|f| {
        let self_path = S::new(Expr::Path {
            path: ModulePath { segments: vec![self_name.clone()], span: c.span },
            span: c.span,
        }, c.span);
        let field_access = S::new(Expr::Field { expr: Box::new(self_path), field: f.name.clone(), span: c.span }, c.span);
        Stmt::Val {
            pattern: S::new(Pattern::Ident { name: f.name.clone(), span: f.name.span }, f.name.span),
            ty: None,
            value: field_access,
            span: c.span,
        }
    }).collect();

    // BACKLOG item 173: a computed body may also reference *other*
    // computed properties on the same record, not just stored fields —
    // bind exactly the ones it actually references (via the conservative
    // textwalk above), not every computed property regardless of use.
    // Each binding is `val other = self.other`, and since `other` is a
    // real computed name, HIR's own `Expr::Field` computed-field fallback
    // (`crates/hir/src/lower.rs`) already lowers that read to a real call
    // to `TypeName.other(self)` — so this needs zero new evaluation logic,
    // the same "destructure into locals" reuse the stored-field prelude
    // above already relies on. Deliberately excludes `c`'s own name: a
    // computed property directly referencing itself would just re-call the
    // method currently being defined (guaranteed infinite recursion), so a
    // self-reference is left unbound here and instead fails as an ordinary
    // E0206 undefined-name error. A genuine *cycle* between two different
    // computed properties (A referencing B, B referencing A) isn't
    // statically caught — same as any other mutual-recursion-without-a-
    // base-case in this language; only actually recurses forever if the
    // cyclic accessor is ever called.
    let mut referenced = HashSet::new();
    collect_referenced_idents(&c.body, &mut referenced);
    for other in all_computed {
        if other.name.node != c.name.node && referenced.contains(&other.name.node) {
            let self_path = S::new(Expr::Path {
                path: ModulePath { segments: vec![self_name.clone()], span: c.span },
                span: c.span,
            }, c.span);
            let field_access = S::new(Expr::Field { expr: Box::new(self_path), field: other.name.clone(), span: c.span }, c.span);
            stmts.push(Stmt::Val {
                pattern: S::new(Pattern::Ident { name: other.name.clone(), span: other.name.span }, other.name.span),
                ty: None,
                value: field_access,
                span: c.span,
            });
        }
    }

    stmts.push(Stmt::Expr { expr: c.body.clone(), span: c.body.span });
    let body = S::new(Expr::Block { stmts, span: c.span }, c.span);

    FnDecl {
        is_async: false,
        is_pub: true,
        name: c.name.clone(),
        // Empty, not `type_params.to_vec()` — matching every other in-body
        // method (BACKLOG item 150): the record's own type params are
        // introduced once, by the synthesized `ImplDecl` this method joins
        // (`type_params: d.type_params.clone()` at the `parse_decl` call
        // site), not re-declared per method.
        type_params: Vec::new(),
        params: vec![self_param],
        ret_ty: Some(c.ty.clone()),
        effects: None,
        body: Some(body),
        is_extern: false,
        export_name: None,
        span: c.span,
    }
}

/// Returns the record's plain fields/computed fields plus any in-body `fn`
/// methods (spec §8.5, BACKLOG item 150) — parsed with the exact same
/// `parse_fn_decl` helper an ordinary `impl X { ... }` block's own method
/// loop already uses (`parse_impl_decl` below), so a method here needs the
/// identical explicit `name: Type` signature for every parameter that
/// convention already requires (including any parameter meant as the
/// receiver — this language has no `self`-keyword/implicit-receiver sugar
/// anywhere, confirmed via existing `impl` blocks, which always spell the
/// receiver as an ordinary named-and-typed parameter; this slice doesn't
/// invent one either, only in-body `fn` parsing itself).
fn parse_record_type_def(cur: &mut Cursor<'_>) -> Result<(RecordTypeDef, Vec<FnDecl>), ParseError> {
    let start = cur.expect(&Token::LBrace)?;
    let mut fields   = Vec::new();
    let mut computed = Vec::new();
    let mut methods  = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let _field_span = cur.peek_span();

        // In-body `fn`/`pub fn`/`async fn` method (BACKLOG item 150) —
        // desugars to a method on a synthesized `impl` block, so it's
        // checked before the `computed`/plain-field arms below.
        if matches!(cur.peek(), Some(Token::Fn) | Some(Token::Async))
            || (cur.peek() == Some(&Token::Pub) && matches!(cur.peek2(), Some(Token::Fn) | Some(Token::Async)))
        {
            let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();
            methods.push(parse_fn_decl(cur, is_pub)?);
            cur.eat(|t| matches!(t, Token::Comma));
            continue;
        }

        // `computed fieldName: Type = expr`
        if let Some(Token::Ident(s)) = cur.peek() {
            if *s == "computed" {
                cur.bump();
                let (fname, fspan) = cur.expect_ident()?;
                cur.expect(&Token::Colon)?;
                let ty = parse_type(cur)?;
                cur.expect(&Token::Eq)?;
                let body = parse_expr(cur)?;
                let span = fspan.to(body.span);
                computed.push(ComputedFieldDef { name: S::new(fname, fspan), ty, body, span });
                cur.eat(|t| matches!(t, Token::Comma));
                continue;
            }
        }

        let (fname, fspan) = cur.expect_ident()?;
        let optional = cur.eat(|t| matches!(t, Token::Question)).is_some();
        cur.expect(&Token::Colon)?;
        let ty = parse_type(cur)?;
        let span = fspan.to(ty.span);
        fields.push(RecordFieldDef { name: S::new(fname, fspan), ty, optional, span });
        cur.eat(|t| matches!(t, Token::Comma));
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok((RecordTypeDef { fields, computed, span }, methods))
}

fn parse_sum_variants(cur: &mut Cursor<'_>) -> Result<Vec<SumVariant>, ParseError> {
    let mut variants = Vec::new();

    while cur.peek() == Some(&Token::Bar) {
        cur.bump(); // eat `|`
        let (name, name_span) = cur.expect_ident()?;
        let mut fields = Vec::new();

        if cur.eat(|t| matches!(t, Token::LParen)).is_some() {
            while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
                // Named: `radius: Float`  or positional: `Float`
                let name = if let Some(Token::Ident(_)) = cur.peek() {
                    if cur.peek2() == Some(&Token::Colon) {
                        let (n, ns) = cur.expect_ident()?;
                        cur.bump(); // eat `:`
                        Some(S::new(n, ns))
                    } else { None }
                } else { None };
                let ty = parse_type(cur)?;
                let span = name.as_ref().map(|n| n.span).unwrap_or(ty.span).to(ty.span);
                fields.push(VariantField { name, ty, span });
                if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
            }
            cur.expect(&Token::RParen)?;
        }

        let span = name_span; // rough
        variants.push(SumVariant { name: S::new(name, name_span), fields, span });
    }

    Ok(variants)
}

/// Parse sum variants without a leading `|`: `Red | Green | Blue`
fn parse_sum_variants_no_leading_bar(cur: &mut Cursor<'_>) -> Result<Vec<SumVariant>, ParseError> {
    let mut variants = Vec::new();
    loop {
        let (name, name_span) = cur.expect_ident()?;
        let mut fields = Vec::new();

        if cur.eat(|t| matches!(t, Token::LParen)).is_some() {
            while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
                let fname = if let Some(Token::Ident(_)) = cur.peek() {
                    if cur.peek2() == Some(&Token::Colon) {
                        let (n, ns) = cur.expect_ident()?;
                        cur.bump(); // eat `:`
                        Some(S::new(n, ns))
                    } else { None }
                } else { None };
                let ty = parse_type(cur)?;
                let span = fname.as_ref().map(|n| n.span).unwrap_or(ty.span).to(ty.span);
                fields.push(VariantField { name: fname, ty, span });
                if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
            }
            cur.expect(&Token::RParen)?;
        }

        variants.push(SumVariant { name: S::new(name, name_span), fields, span: name_span });
        if cur.eat(|t| matches!(t, Token::Bar)).is_none() { break; }
    }
    Ok(variants)
}

// ------------------------------------------------------------------ //
// Val / Var
// ------------------------------------------------------------------ //

fn parse_val_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<ValDecl, ParseError> {
    let start = cur.expect(&Token::Val)?;
    let pattern = parse_pattern(cur)?;
    let ty = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
        Some(parse_type(cur)?)
    } else {
        None
    };
    cur.expect(&Token::Eq)?;
    let value = parse_expr(cur)?;
    let span = start.to(value.span);
    Ok(ValDecl { is_pub, pattern, ty, value, span })
}

fn parse_var_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<VarDecl, ParseError> {
    let start = cur.expect(&Token::Var)?;
    let (name, name_span) = cur.expect_ident()?;
    let ty = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
        Some(parse_type(cur)?)
    } else {
        None
    };
    cur.expect(&Token::Eq)?;
    let value = parse_expr(cur)?;
    let span = start.to(value.span);
    Ok(VarDecl { is_pub, name: S::new(name, name_span), ty, value, span })
}

// ------------------------------------------------------------------ //
// Trait / Impl
// ------------------------------------------------------------------ //

fn parse_trait_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<TraitDecl, ParseError> {
    let start = cur.expect(&Token::Trait)?;
    let (name, name_span) = cur.expect_ident()?;
    let type_params = parse_type_params(cur)?;
    cur.expect(&Token::LBrace)?;
    let mut methods = Vec::new();
    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();
        methods.push(parse_fn_decl(cur, is_pub)?);
    }
    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(TraitDecl { is_pub, name: S::new(name, name_span), type_params, methods, span })
}

fn parse_impl_decl(cur: &mut Cursor<'_>) -> Result<ImplDecl, ParseError> {
    let start = cur.expect(&Token::Impl)?;
    let type_params = parse_type_params(cur)?;

    let first = parse_module_path(cur)?;

    // `impl Trait for Type` vs `impl Type`
    let (trait_path, type_path) = if cur.eat(|t| matches!(t, Token::For)).is_some() {
        let tp = parse_module_path(cur)?;
        (Some(first), tp)
    } else {
        (None, first)
    };

    cur.expect(&Token::LBrace)?;
    let mut methods = Vec::new();
    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();
        methods.push(parse_fn_decl(cur, is_pub)?);
    }
    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(ImplDecl { trait_path, type_path, type_params, methods, span })
}

// ------------------------------------------------------------------ //
// State machine
// ------------------------------------------------------------------ //

fn parse_statemachine(cur: &mut Cursor<'_>) -> Result<StateMachineDecl, ParseError> {
    let start = cur.expect(&Token::StateMachine)?;
    let (name, name_span) = cur.expect_ident()?;
    cur.expect(&Token::LBrace)?;

    let mut states      = Vec::new();
    let mut transitions = Vec::new();
    let mut on_enter    = Vec::new();
    let mut invariants  = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let (kw, kw_span) = cur.expect_ident()?;
        match kw.as_str() {
            "states" => {
                cur.expect(&Token::Colon)?;
                loop {
                    let (s, ss) = cur.expect_ident()?;
                    states.push(S::new(s, ss));
                    if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
                    if cur.peek().map(|t| matches!(t, Token::Ident(_))).unwrap_or(false)
                        && matches!(cur.peek(), Some(Token::Ident(s)) if
                            ["transitions","on_enter","invariant"].contains(&s.as_ref()))
                    { break; }
                }
            }
            "transitions" => {
                cur.expect(&Token::Colon)?;
                while let Some(Token::Ident(_)) = cur.peek() {
                    // Peek before consuming — section keywords end the transitions block.
                    if let Some(Token::Ident(s)) = cur.peek() {
                        if ["on_enter", "invariant"].contains(&s.as_ref()) { break; }
                    }
                    let (from, from_span) = cur.expect_ident()?;
                    // `→` arrow may be tokenised as `->` or we allow `→` as an ident
                    // Support both Token::Arrow and the Unicode arrow
                    if cur.peek() == Some(&Token::Arrow) {
                        cur.bump();
                    } else {
                        let (arrow, arrow_span) = cur.expect_ident()?;
                        if arrow != "→" {
                            return Err(ParseError {
                                kind: ParseErrorKind::Expected { expected: "→".into(), found: arrow },
                                span: arrow_span,
                            });
                        }
                    }
                    let (to, to_span) = cur.expect_ident()?;
                    cur.expect(&Token::Colon)?;
                    let (event, event_span) = cur.expect_ident()?;
                    let mut params = Vec::new();
                    if cur.eat(|t| matches!(t, Token::LParen)).is_some() {
                        while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
                            params.push(parse_fn_param(cur)?);
                            if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
                        }
                        cur.expect(&Token::RParen)?;
                    }
                    let span = from_span.to(event_span);
                    transitions.push(Transition {
                        from: S::new(from, from_span),
                        to:   S::new(to, to_span),
                        event: S::new(event, event_span),
                        params,
                        span,
                    });
                }
            }
            "on_enter" => {
                let (state, state_span) = cur.expect_ident()?;
                cur.expect(&Token::Colon)?;
                let body = parse_expr(cur)?;
                let span = kw_span.to(body.span);
                on_enter.push(OnEnterHook { state: S::new(state, state_span), body, span });
            }
            "invariant" => {
                let (state, state_span) = cur.expect_ident()?;
                cur.expect(&Token::Colon)?;
                let cond = parse_expr(cur)?;
                let span = kw_span.to(cond.span);
                invariants.push(Invariant { state: S::new(state, state_span), cond, span });
            }
            other => return Err(ParseError {
                kind: ParseErrorKind::Custom(format!("unexpected key `{other}` in statemachine")),
                span: kw_span,
            }),
        }
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(StateMachineDecl {
        name: S::new(name, name_span),
        states,
        transitions,
        on_enter,
        invariants,
        span,
    })
}

// ------------------------------------------------------------------ //
// Migration
// ------------------------------------------------------------------ //

fn parse_migration(cur: &mut Cursor<'_>) -> Result<MigrationDecl, ParseError> {
    let start = cur.expect(&Token::Migration)?;
    let (name_tok, _) = cur.bump().ok_or(ParseError {
        kind: ParseErrorKind::UnexpectedEof,
        span: start,
    })?;
    let name = if let Token::StringLit(s) = name_tok { s.to_string() } else {
        return Err(ParseError { kind: ParseErrorKind::Expected { expected: "migration name string".into(), found: format!("{name_tok:?}") }, span: start });
    };

    cur.expect(&Token::LBrace)?;
    let mut description = None;
    let mut up   = Vec::new();
    let mut down = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let (kw, kw_span) = cur.expect_ident()?;
        match kw.as_str() {
            "description" => {
                cur.expect(&Token::Eq)?;
                let (tok, _) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: kw_span })?;
                if let Token::StringLit(s) = tok { description = Some(s.to_string()); }
            }
            "up" => {
                cur.expect(&Token::LBrace)?;
                while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
                    up.push(parse_migration_op(cur)?);
                }
                cur.expect(&Token::RBrace)?;
            }
            "down" => {
                cur.expect(&Token::LBrace)?;
                while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
                    down.push(parse_migration_op(cur)?);
                }
                cur.expect(&Token::RBrace)?;
            }
            other => return Err(ParseError {
                kind: ParseErrorKind::Custom(format!("unexpected key `{other}` in migration")),
                span: kw_span,
            }),
        }
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(MigrationDecl { name, description, up, down, span })
}

/// Parse a single migration operation inside an `up`/`down` block.
///
/// Grammar (keyword-led blocks):
/// ```text
///   createTable NAME { COL (, COL)* ,? }
///   alterTable  NAME { ALTEROP (,? ALTEROP)* }
///   dropTable   NAME
///   createIndex NAME on NAME [ NAME (, NAME)* ]?
///   dropIndex   NAME
///   rawSql      "..."
///
///   COL     := NAME : TYPE MODIFIER*
///   MODIFIER:= primaryKey | unique | nullable | default EXPR
///   ALTEROP := addColumn COL
///            | dropColumn NAME
///            | foreignKey NAME references NAME (onDelete ACTION)?
///   ACTION  := cascade | setNull | restrict | noAction
///   NAME    := IDENT | STRING
/// ```
fn parse_migration_op(cur: &mut Cursor<'_>) -> Result<MigrationOp, ParseError> {
    let (kw, kw_span) = cur.expect_ident()?;
    match kw.as_str() {
        "createTable" => {
            let (name, _) = parse_db_name(cur)?;
            cur.expect(&Token::LBrace)?;
            let mut columns = Vec::new();
            while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
                columns.push(parse_column_def(cur)?);
                if cur.peek() == Some(&Token::Comma) { cur.bump(); }
            }
            let end = cur.expect(&Token::RBrace)?;
            Ok(MigrationOp::CreateTable { name, columns, span: kw_span.to(end) })
        }
        "alterTable" => {
            let (name, _) = parse_db_name(cur)?;
            cur.expect(&Token::LBrace)?;
            let mut ops = Vec::new();
            while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
                ops.push(parse_alter_op(cur)?);
                if cur.peek() == Some(&Token::Comma) { cur.bump(); }
            }
            let end = cur.expect(&Token::RBrace)?;
            Ok(MigrationOp::AlterTable { name, ops, span: kw_span.to(end) })
        }
        "dropTable" => {
            let (name, nspan) = parse_db_name(cur)?;
            Ok(MigrationOp::DropTable { name, span: kw_span.to(nspan) })
        }
        "createIndex" => {
            let (name, _) = parse_db_name(cur)?;
            cur.expect(&Token::On)?;
            let (table, tspan) = parse_db_name(cur)?;
            let mut columns = Vec::new();
            let mut end = tspan;
            if cur.peek() == Some(&Token::LBracket) {
                cur.expect(&Token::LBracket)?;
                while cur.peek() != Some(&Token::RBracket) && !cur.at_end() {
                    let (c, _) = parse_db_name(cur)?;
                    columns.push(c);
                    if cur.peek() == Some(&Token::Comma) { cur.bump(); }
                }
                end = cur.expect(&Token::RBracket)?;
            }
            Ok(MigrationOp::CreateIndex { name, table, columns, span: kw_span.to(end) })
        }
        "dropIndex" => {
            let (name, nspan) = parse_db_name(cur)?;
            Ok(MigrationOp::DropIndex { name, span: kw_span.to(nspan) })
        }
        "rawSql" => {
            let (tok, tspan) = cur.bump().ok_or(ParseError {
                kind: ParseErrorKind::UnexpectedEof, span: kw_span })?;
            if let Token::StringLit(s) = tok {
                Ok(MigrationOp::RawSql { sql: s.to_string(), span: kw_span.to(tspan) })
            } else {
                Err(ParseError {
                    kind: ParseErrorKind::Expected {
                        expected: "SQL string literal after `rawSql`".into(),
                        found: format!("{tok:?}") },
                    span: tspan })
            }
        }
        other => Err(ParseError {
            kind: ParseErrorKind::Custom(format!(
                "unknown migration operation `{other}` — expected one of: \
                 createTable, alterTable, dropTable, createIndex, dropIndex, rawSql")),
            span: kw_span,
        }),
    }
}

/// A table/column/index name: a bare identifier or a string literal.
fn parse_db_name(cur: &mut Cursor<'_>) -> Result<(String, certo_ast::span::Span), ParseError> {
    if let Some(Token::StringLit(_)) = cur.peek() {
        if let Some((Token::StringLit(s), sp)) = cur.bump() {
            return Ok((s.to_string(), sp));
        }
    }
    cur.expect_ident()
}

/// `NAME : TYPE MODIFIER*`
fn parse_column_def(cur: &mut Cursor<'_>) -> Result<ColumnDef, ParseError> {
    let (name, nspan) = parse_db_name(cur)?;
    cur.expect(&Token::Colon)?;
    let ty = parse_type(cur)?;
    let mut primary_key = false;
    let mut nullable    = false;
    let mut unique      = false;
    let mut default     = None;
    let mut end         = ty.span;
    // Only consume recognised modifier keywords; stop at anything else (a comma,
    // closing brace, or the next alter-op keyword like `dropColumn`).
    while matches!(cur.peek(),
        Some(Token::Ident(s)) if matches!(*s, "primaryKey" | "unique" | "nullable" | "default"))
    {
        let (m, mspan) = cur.expect_ident()?;
        match m.as_str() {
            "primaryKey" => { primary_key = true; end = mspan; }
            "unique"     => { unique = true;      end = mspan; }
            "nullable"   => { nullable = true;    end = mspan; }
            "default"    => {
                let e = parse_expr(cur)?;
                end = e.span;
                default = Some(e);
            }
            _ => unreachable!("guarded by matches! above"),
        }
    }
    Ok(ColumnDef { name, ty, primary_key, nullable, unique, default, span: nspan.to(end) })
}

/// `addColumn COL | dropColumn NAME | foreignKey NAME references NAME (onDelete ACTION)?`
fn parse_alter_op(cur: &mut Cursor<'_>) -> Result<AlterOp, ParseError> {
    let (kw, kw_span) = cur.expect_ident()?;
    match kw.as_str() {
        "addColumn" => Ok(AlterOp::AddColumn { def: parse_column_def(cur)? }),
        "dropColumn" => {
            let (name, nspan) = parse_db_name(cur)?;
            Ok(AlterOp::DropColumn { name, span: kw_span.to(nspan) })
        }
        "foreignKey" => {
            let (column, _) = parse_db_name(cur)?;
            let (r, rspan) = cur.expect_ident()?;
            if r != "references" {
                return Err(ParseError {
                    kind: ParseErrorKind::Expected { expected: "`references`".into(), found: r },
                    span: rspan });
            }
            let (references, refspan) = parse_db_name(cur)?;
            let mut on_delete = FkAction::NoAction;
            let mut end = refspan;
            if matches!(cur.peek(), Some(Token::Ident(s)) if *s == "onDelete") {
                cur.expect_ident()?; // consume `onDelete`
                let (act, aspan) = cur.expect_ident()?;
                on_delete = match act.as_str() {
                    "cascade"  => FkAction::Cascade,
                    "setNull"  => FkAction::SetNull,
                    "restrict" => FkAction::Restrict,
                    "noAction" => FkAction::NoAction,
                    other => return Err(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "unknown onDelete action `{other}` — expected cascade, setNull, restrict, or noAction")),
                        span: aspan,
                    }),
                };
                end = aspan;
            }
            Ok(AlterOp::AddForeignKey { column, references, on_delete, span: kw_span.to(end) })
        }
        other => Err(ParseError {
            kind: ParseErrorKind::Custom(format!(
                "unknown alter operation `{other}` — expected addColumn, dropColumn, or foreignKey")),
            span: kw_span,
        }),
    }
}

// ------------------------------------------------------------------ //
// View
// ------------------------------------------------------------------ //

fn parse_view(cur: &mut Cursor<'_>) -> Result<ViewDecl, ParseError> {
    let start = cur.expect(&Token::View)?;
    let (name, name_span) = cur.expect_ident()?;

    let mut pk: Option<String> = None;
    let mut filter_by: Option<String> = None;
    let mut layout_expr: Option<S<certo_ast::expr::Expr>> = None;
    let mut live: Vec<certo_ast::decl::ValDecl> = Vec::new();

    cur.expect(&Token::LBrace)?;
    loop {
        if cur.peek() == Some(&Token::RBrace) { break; }
        if let Some(Token::Ident(k)) = cur.peek().cloned() {
            let k_str = k.to_string();
            match k_str.as_str() {
                "pk" => {
                    cur.bump();
                    cur.expect(&Token::Eq)?;
                    let (val, _) = cur.expect_ident()?;
                    pk = Some(val);
                    cur.eat(|t| matches!(t, Token::Comma));
                    continue;
                }
                "filter" => {
                    cur.bump();
                    cur.expect(&Token::Eq)?;
                    let (val, _) = cur.expect_ident()?;
                    filter_by = Some(val);
                    cur.eat(|t| matches!(t, Token::Comma));
                    continue;
                }
                "layout" => {
                    cur.bump();
                    cur.expect(&Token::Eq)?;
                    layout_expr = Some(parse_expr(cur)?);
                    cur.eat(|t| matches!(t, Token::Comma));
                    continue;
                }
                // `live` is a contextual keyword (same soft-keyword pattern
                // as `pk`/`filter`/`layout` above, not a globally reserved
                // lexer token) recognized only here, inside a view body —
                // BACKLOG item 88. `live val x = expr` reuses the ordinary
                // `val` parser unchanged; there is deliberately no new
                // syntax for the binding itself, only for the `live`
                // prefix that marks it. Real reactive wiring (a query
                // DSL, a push/streaming transport) is a separate, much
                // larger, unbuilt gap — this only parses the declaration
                // into `ViewDecl.live` so codegen can at least see it and
                // say so honestly, instead of it being a parse error.
                "live" if cur.peek2() == Some(&Token::Val) => {
                    cur.bump();
                    live.push(parse_val_decl(cur, false)?);
                    cur.eat(|t| matches!(t, Token::Comma));
                    continue;
                }
                _ => {}
            }
        }
        // Unknown key or non-ident token: parse as expression and use as layout
        layout_expr = Some(parse_expr(cur)?);
        cur.eat(|t| matches!(t, Token::Comma));
    }
    let end = cur.expect(&Token::RBrace)?;
    let layout = layout_expr.ok_or_else(|| ParseError {
        kind: crate::error::ParseErrorKind::Custom("view missing layout".into()),
        span: name_span,
    })?;
    let span = start.to(end);
    Ok(ViewDecl { name: S::new(name, name_span), live, layout, pk, filter_by, span })
}

fn parse_form(cur: &mut Cursor<'_>) -> Result<FormDecl, ParseError> {
    let start = cur.expect(&Token::Form)?;
    let (name, name_span) = cur.expect_ident()?;
    // Optional `-> ModulePath` target
    let target = if cur.peek() == Some(&Token::Arrow) {
        cur.bump();
        parse_module_path(cur)?
    } else {
        use certo_ast::{types::ModulePath, span::Span};
        ModulePath { segments: vec![], span: Span { start: start.start, end: start.end } }
    };
    // Optional body block — parse as key: value pairs
    let mut fields = vec![];
    let mut pk: Option<String> = None;
    let mut on_submit = None;
    let mut on_success = None;
    let end_span;
    if cur.peek() == Some(&Token::LBrace) {
        let _brace_start = cur.expect(&Token::LBrace)?;
        loop {
            if cur.peek() == Some(&Token::RBrace) { break; }
            let (key, key_span) = cur.expect_ident()?;
            match key.as_str() {
                "pk" => {
                    cur.expect(&Token::Colon)?;
                    let (val, _) = cur.expect_ident()?;
                    pk = Some(val);
                }
                "onSubmit" => {
                    cur.expect(&Token::Colon)?;
                    on_submit = Some(parse_expr(cur)?);
                }
                "onSuccess" => {
                    cur.expect(&Token::Colon)?;
                    on_success = Some(parse_expr(cur)?);
                }
                _ => {
                    // field declaration
                    cur.expect(&Token::Colon)?;
                    let field_type = Some(parse_expr(cur)?);
                    fields.push(FormField {
                        name: S::new(key, key_span),
                        label: None, placeholder: None,
                        field_type, options: None, rows: None,
                        span: key_span,
                    });
                }
            }
            // optional comma / newline separator
            cur.eat(|t| matches!(t, Token::Comma));
        }
        end_span = cur.expect(&Token::RBrace)?;
    } else {
        end_span = name_span;
    }
    let span = start.to(end_span);
    Ok(FormDecl { name: S::new(name, name_span), target, fields, pk, on_submit, on_success, span })
}

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

fn parse_test_decl(cur: &mut Cursor<'_>) -> Result<TestDecl, ParseError> {
    let start = cur.expect(&Token::Test)?;
    let (tok, _) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let name = if let Token::StringLit(s) = tok { s.to_string() } else { String::new() };
    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(TestDecl { name, body, span })
}

fn parse_property_decl(cur: &mut Cursor<'_>) -> Result<PropertyDecl, ParseError> {
    let start = cur.expect(&Token::Property)?;
    let (tok, _) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let name = if let Token::StringLit(s) = tok { s.to_string() } else { String::new() };

    // Optional typed parameter list: `property "name"(x: Int, y: Text) { .. }`.
    // A bare `property "name" { .. }` (no parens) still parses — same as before
    // this was added — and just means "no generated inputs, run once".
    let mut params = Vec::new();
    if cur.eat(|t| matches!(t, Token::LParen)).is_some() {
        while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
            params.push(parse_fn_param(cur)?);
            if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        }
        cur.expect(&Token::RParen)?;
    }

    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(PropertyDecl { name, params, body, span })
}

fn parse_dbtest_decl(cur: &mut Cursor<'_>) -> Result<DbTestDecl, ParseError> {
    let start = cur.expect(&Token::DbTest)?;
    let (tok, _) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let name = if let Token::StringLit(s) = tok { s.to_string() } else { String::new() };
    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(DbTestDecl { name, body, span })
}

// ------------------------------------------------------------------ //
// Import
// ------------------------------------------------------------------ //

fn parse_import_decl(cur: &mut Cursor<'_>) -> Result<ImportDecl, ParseError> {
    let start = cur.expect(&Token::Import)?;
    // Parse a dot-separated path: `Stdlib.Text`, `Stdlib.Math`, etc.
    let (first, first_span) = cur.expect_ident()?;
    let mut path = vec![first.to_string()];
    let mut end = first_span;
    while cur.eat(|t| matches!(t, Token::Dot)).is_some() {
        let (seg, seg_span) = cur.expect_ident()?;
        path.push(seg.to_string());
        end = seg_span;
    }
    Ok(ImportDecl { path, span: start.to(end) })
}

// ------------------------------------------------------------------ //
// Constraint
// ------------------------------------------------------------------ //

fn parse_constraint_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<ConstraintDecl, ParseError> {
    let start = cur.expect(&Token::Constraint)?;
    let (name, name_span) = cur.expect_ident()?;
    cur.expect(&Token::Eq)?;
    let body = parse_expr(cur)?;
    let span = start.to(body.span);
    Ok(ConstraintDecl { is_pub, name: S::new(name, name_span), body, span })
}

// ------------------------------------------------------------------ //
// Temporal
// ------------------------------------------------------------------ //

fn parse_temporal_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<TemporalDecl, ParseError> {
    let start = cur.expect(&Token::Temporal)?;
    let (name, name_span) = cur.expect_ident()?;
    cur.expect(&Token::Eq)?;
    let body = parse_expr(cur)?;
    let span = start.to(body.span);
    Ok(TemporalDecl { is_pub, name: S::new(name, name_span), body, span })
}

// ------------------------------------------------------------------ //
// Validator
// ------------------------------------------------------------------ //

fn parse_validator_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<ValidatorDecl, ParseError> {
    let start = cur.expect(&Token::Validator)?;
    let (name, name_span) = cur.expect_ident()?;

    cur.expect(&Token::For)?;
    let entity = parse_type(cur)?;

    cur.expect(&Token::Errors)?;
    let errors = parse_type(cur)?;

    // Optional trigger annotation — appears before the opening brace
    let trigger = if cur.peek() == Some(&Token::Trigger) {
        Some(parse_trigger_decl(cur)?)
    } else {
        None
    };

    cur.expect(&Token::LBrace)?;

    // Optional context block
    let context = if cur.peek() == Some(&Token::Context) {
        parse_context_block(cur)?
    } else {
        Vec::new()
    };

    // Zero or more rule declarations
    let mut rules = Vec::new();
    while cur.peek() == Some(&Token::Rule) {
        rules.push(parse_rule_decl(cur)?);
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(ValidatorDecl { is_pub, name: S::new(name, name_span), entity, errors, trigger, context, rules, span })
}

fn parse_trigger_decl(cur: &mut Cursor<'_>) -> Result<TriggerDecl, ParseError> {
    let start = cur.expect(&Token::Trigger)?;
    cur.expect(&Token::On)?;

    let (op_str, op_span) = cur.expect_ident()?;
    let op = match op_str.as_str() {
        "Insert" => TriggerOp::Insert,
        "Update" => TriggerOp::Update,
        other => return Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "Insert or Update".into(), found: other.to_string() },
            span: op_span,
        }),
    };

    let condition = if cur.eat(|t| matches!(t, Token::When)).is_some() {
        let (field, field_span) = cur.expect_ident()?;
        let cond_op = if cur.eat(|t| matches!(t, Token::EqEq)).is_some() {
            TriggerCondOp::Eq
        } else {
            cur.expect(&Token::NotEq)?;
            TriggerCondOp::NotEq
        };
        // Use a restricted parser so that an uppercase enum variant like `Submitted`
        // is NOT treated as a record literal — the `{` that follows belongs to the
        // validator body, not to the condition value.
        let value = parse_trigger_value(cur)?;
        let cond_span = field_span.to(value.span);
        Some(TriggerCondition {
            field: S::new(field, field_span),
            op:    cond_op,
            value,
            span:  cond_span,
        })
    } else {
        None
    };

    let span = start.to(cur.peek_span());
    Ok(TriggerDecl { op, condition, span })
}

/// Parse a trigger condition value as a restricted path/field expression.
/// Deliberately avoids `parse_expr` so that an uppercase name like `Submitted`
/// followed by `{` is NOT parsed as a record literal (the `{` belongs to the
/// validator body, not the condition value).
fn parse_trigger_value(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let path = parse_module_path(cur)?;
    let span = path.span;
    let mut expr = S::new(Expr::Path { path, span }, span);

    // Follow lowercase field accesses (e.g. `OLD.status`)
    loop {
        if cur.peek() != Some(&Token::Dot) { break; }
        let starts_lower = match cur.peek2() {
            Some(Token::Ident(next)) => next.chars().next().map(|c| c.is_lowercase()).unwrap_or(false),
            _ => false,
        };
        if !starts_lower { break; }
        cur.bump(); // consume the dot
        let (field, field_span) = cur.expect_ident()?;
        let new_span = expr.span.to(field_span);
        expr = S::new(
            Expr::Field { expr: Box::new(expr), field: S::new(field, field_span), span: new_span },
            new_span,
        );
    }

    Ok(expr)
}

fn parse_context_block(cur: &mut Cursor<'_>) -> Result<Vec<ContextField>, ParseError> {
    cur.expect(&Token::Context)?;
    cur.expect(&Token::LBrace)?;
    let mut fields = Vec::new();
    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let (name, name_span) = cur.expect_ident()?;
        cur.expect(&Token::Colon)?;
        let type_ref = parse_type(cur)?;
        let loaded_by = if cur.eat(|t| matches!(t, Token::Loaded)).is_some() {
            // `loaded by expr` — `by` is an identifier, not a keyword
            let (by_str, by_span) = cur.expect_ident()?;
            if by_str != "by" {
                return Err(ParseError {
                    kind: ParseErrorKind::Expected { expected: "by".into(), found: by_str },
                    span: by_span,
                });
            }
            Some(parse_expr(cur)?)
        } else {
            None
        };
        let span = name_span.to(type_ref.span);
        fields.push(ContextField { name: S::new(name, name_span), type_ref, loaded_by, span });
        cur.eat(|t| matches!(t, Token::Comma));
    }
    cur.expect(&Token::RBrace)?;
    Ok(fields)
}

fn parse_rule_decl(cur: &mut Cursor<'_>) -> Result<RuleDecl, ParseError> {
    let start = cur.expect(&Token::Rule)?;
    let (name, name_span) = cur.expect_ident()?;
    cur.expect(&Token::LBrace)?;

    let mut after     = Vec::new();
    let mut overrides = None;
    let mut priority  = None;

    // `after`, `overrides`, `priority` can appear in any order before `require`
    loop {
        match cur.peek() {
            Some(Token::After) => {
                cur.bump();
                let (rname, rspan) = cur.expect_ident()?;
                after.push(S::new(rname, rspan));
            }
            Some(Token::Overrides) => {
                cur.bump();
                let (rname, rspan) = cur.expect_ident()?;
                overrides = Some(S::new(rname, rspan));
            }
            Some(Token::Priority) => {
                cur.bump();
                let (tok, tok_span) = cur.bump().ok_or(ParseError {
                    kind: ParseErrorKind::UnexpectedEof,
                    span: start,
                })?;
                if let Token::Integer(s) = tok {
                    priority = Some(s.parse::<i64>().unwrap_or(0));
                } else {
                    return Err(ParseError {
                        kind: ParseErrorKind::Expected { expected: "integer".into(), found: format!("{tok:?}") },
                        span: tok_span,
                    });
                }
            }
            Some(Token::Require) | _ => break,
        }
    }

    cur.expect(&Token::Require)?;
    let require = parse_expr(cur)?;
    cur.expect(&Token::Else)?;
    let else_ = parse_expr(cur)?;

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(RuleDecl { name: S::new(name, name_span), after, overrides, priority, require, else_, span })
}

// ------------------------------------------------------------------ //
// Test blocks
// ------------------------------------------------------------------ //

/// Parse a qualified identifier: `Ident ("." Ident)*`
fn parse_qualified_ident(cur: &mut Cursor<'_>) -> Result<Vec<S<String>>, ParseError> {
    let (first, first_span) = cur.expect_ident()?;
    let mut segments = vec![S::new(first, first_span)];
    while cur.peek() == Some(&Token::Dot) {
        // Only consume the dot if the next token after it is an identifier
        if matches!(cur.peek2(), Some(Token::Ident(_))) {
            cur.bump(); // eat `.`
            let (seg, seg_span) = cur.expect_ident()?;
            segments.push(S::new(seg, seg_span));
        } else {
            break;
        }
    }
    Ok(segments)
}

fn parse_test_expectation(cur: &mut Cursor<'_>) -> Result<TestExpectation, ParseError> {
    let (kw, kw_span) = cur.expect_ident()?;
    match kw.as_str() {
        "pass" => Ok(TestExpectation::Pass),
        "fail" => {
            let with = if cur.eat(|t| matches!(t, Token::With)).is_some() {
                Some(parse_expr(cur)?)
            } else {
                None
            };
            Ok(TestExpectation::Fail { with })
        }
        other => Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "pass or fail".into(), found: other.to_string() },
            span: kw_span,
        }),
    }
}

/// Parse the body `{ entity: expr  context: expr  expect: expectation }` shared by both test blocks.
fn parse_test_body(cur: &mut Cursor<'_>) -> Result<(S<Expr>, S<Expr>, TestExpectation), ParseError> {
    cur.expect(&Token::LBrace)?;
    let mut entity_expr  = None;
    let mut context_expr = None;
    let mut expect_val   = None;

    loop {
        if cur.peek() == Some(&Token::RBrace) { break; }

        // Key is either a keyword (context) or an identifier (entity, expect)
        let key = match cur.peek() {
            Some(Token::Context) => {
                cur.bump();
                "context".to_string()
            }
            Some(Token::Ident(_)) => {
                let (k, _) = cur.expect_ident()?;
                k
            }
            _ => break,
        };

        cur.expect(&Token::Colon)?;

        match key.as_str() {
            "entity"  => { entity_expr  = Some(parse_expr(cur)?); }
            "context" => { context_expr = Some(parse_expr(cur)?); }
            "expect"  => { expect_val   = Some(parse_test_expectation(cur)?); }
            other => return Err(ParseError {
                kind: ParseErrorKind::Custom(format!("unexpected key `{other}` in test block")),
                span: cur.peek_span(),
            }),
        }
        cur.eat(|t| matches!(t, Token::Comma));
    }

    cur.expect(&Token::RBrace)?;

    let entity = entity_expr.ok_or_else(|| ParseError {
        kind: ParseErrorKind::Custom("test block missing `entity`".into()),
        span: cur.peek_span(),
    })?;
    let context = context_expr.ok_or_else(|| ParseError {
        kind: ParseErrorKind::Custom("test block missing `context`".into()),
        span: cur.peek_span(),
    })?;
    let expect = expect_val.ok_or_else(|| ParseError {
        kind: ParseErrorKind::Custom("test block missing `expect`".into()),
        span: cur.peek_span(),
    })?;

    Ok((entity, context, expect))
}

fn parse_rule_test_decl(cur: &mut Cursor<'_>) -> Result<RuleTestDecl, ParseError> {
    let start = cur.expect(&Token::RuleTest)?;
    let validator = parse_qualified_ident(cur)?;

    let (tok, tok_span) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let label = if let Token::StringLit(s) = tok { s.to_string() } else {
        return Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "test label string".into(), found: format!("{tok:?}") },
            span: tok_span,
        });
    };

    let (entity, context, expect) = parse_test_body(cur)?;
    let span = start.to(cur.peek_span());
    Ok(RuleTestDecl { validator, label, entity, context, expect, span })
}

fn parse_validator_test_decl(cur: &mut Cursor<'_>) -> Result<ValidatorTestDecl, ParseError> {
    let start = cur.expect(&Token::ValidatorTest)?;
    let (name, name_span) = cur.expect_ident()?;

    let (tok, tok_span) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let label = if let Token::StringLit(s) = tok { s.to_string() } else {
        return Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "test label string".into(), found: format!("{tok:?}") },
            span: tok_span,
        });
    };

    let (entity, context, expect) = parse_test_body(cur)?;
    let span = start.to(cur.peek_span());
    Ok(ValidatorTestDecl { validator: S::new(name, name_span), label, entity, context, expect, span })
}
