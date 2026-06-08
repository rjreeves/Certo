use certo_ast::span::S;
use certo_ast::decl::*;
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};
use crate::parse_type::{parse_type, parse_type_params, parse_module_path, parse_effect_set};
use crate::parse_expr::{parse_expr, parse_block};
use crate::parse_pattern::parse_pattern;

pub fn parse_decl(cur: &mut Cursor<'_>) -> Result<S<Decl>, ParseError> {
    let span = cur.peek_span();
    let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();

    match cur.peek() {
        Some(Token::Async) | Some(Token::Fn) => {
            let d = parse_fn_decl(cur, is_pub)?;
            let s = d.span;
            Ok(S::new(Decl::Fn(d), s))
        }
        Some(Token::Type) => {
            let d = parse_type_decl(cur, is_pub)?;
            let s = d.span;
            Ok(S::new(Decl::Type(d), s))
        }
        Some(Token::Val) => {
            let d = parse_val_decl(cur, is_pub)?;
            let s = d.span;
            Ok(S::new(Decl::Val(d), s))
        }
        Some(Token::Var) => {
            let d = parse_var_decl(cur, is_pub)?;
            let s = d.span;
            Ok(S::new(Decl::Var(d), s))
        }
        Some(Token::Trait) => {
            let d = parse_trait_decl(cur, is_pub)?;
            let s = d.span;
            Ok(S::new(Decl::Trait(d), s))
        }
        Some(Token::Impl) => {
            let d = parse_impl_decl(cur)?;
            let s = d.span;
            Ok(S::new(Decl::Impl(d), s))
        }
        Some(Token::StateMachine) => {
            let d = parse_statemachine(cur)?;
            let s = d.span;
            Ok(S::new(Decl::StateMachine(d), s))
        }
        Some(Token::Migration) => {
            let d = parse_migration(cur)?;
            let s = d.span;
            Ok(S::new(Decl::Migration(d), s))
        }
        Some(Token::View) => {
            let d = parse_view(cur)?;
            let s = d.span;
            Ok(S::new(Decl::View(d), s))
        }
        Some(Token::Form) => {
            let d = parse_form(cur)?;
            let s = d.span;
            Ok(S::new(Decl::Form(d), s))
        }
        Some(Token::Test) => {
            let d = parse_test_decl(cur)?;
            let s = d.span;
            Ok(S::new(Decl::Test(d), s))
        }
        Some(Token::Property) => {
            let d = parse_property_decl(cur)?;
            let s = d.span;
            Ok(S::new(Decl::Property(d), s))
        }
        Some(Token::DbTest) => {
            let d = parse_dbtest_decl(cur)?;
            let s = d.span;
            Ok(S::new(Decl::DbTest(d), s))
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
    Ok(FnDecl { is_async, is_pub, name: S::new(name, name_span), type_params, params, ret_ty, effects, body, span })
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

fn parse_type_decl(cur: &mut Cursor<'_>, is_pub: bool) -> Result<TypeDecl, ParseError> {
    let start = cur.expect(&Token::Type)?;
    let (name, name_span) = cur.expect_ident()?;
    let type_params = parse_type_params(cur)?;
    cur.expect(&Token::Eq)?;

    let body = if cur.peek() == Some(&Token::LBrace) {
        TypeBody::Record(parse_record_type_def(cur)?)
    } else if cur.peek() == Some(&Token::Bar) {
        TypeBody::Sum(parse_sum_variants(cur)?)
    } else {
        TypeBody::Alias(parse_type(cur)?)
    };

    let span = start.to(cur.peek_span());
    Ok(TypeDecl { is_pub, name: S::new(name, name_span), type_params, body, span })
}

fn parse_record_type_def(cur: &mut Cursor<'_>) -> Result<RecordTypeDef, ParseError> {
    let start = cur.expect(&Token::LBrace)?;
    let mut fields   = Vec::new();
    let mut computed = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let _field_span = cur.peek_span();

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
    Ok(RecordTypeDef { fields, computed, span })
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
                    let (from, from_span) = cur.expect_ident()?;
                    if ["on_enter","invariant"].contains(&from.as_str()) { break; }
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

fn parse_migration_op(cur: &mut Cursor<'_>) -> Result<MigrationOp, ParseError> {
    let span = cur.peek_span();
    // Delegate to expr parser for now — migration ops are method-call expressions
    // that will be lowered in a later pass. This keeps the parser simple.
    let expr = parse_expr(cur)?;
    // Wrap as RawSql placeholder until migration lowering pass is implemented
    Ok(MigrationOp::RawSql { sql: format!("{expr:?}"), span })
}

// ------------------------------------------------------------------ //
// View
// ------------------------------------------------------------------ //

fn parse_view(cur: &mut Cursor<'_>) -> Result<ViewDecl, ParseError> {
    let start = cur.expect(&Token::View)?;
    let (name, name_span) = cur.expect_ident()?;
    let layout = parse_block(cur)?;
    let span = start.to(layout.span);
    Ok(ViewDecl { name: S::new(name, name_span), live: vec![], layout, span })
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
    let mut on_submit = None;
    let mut on_success = None;
    let end_span;
    if cur.peek() == Some(&Token::LBrace) {
        let brace_start = cur.expect(&Token::LBrace)?;
        loop {
            if cur.peek() == Some(&Token::RBrace) { break; }
            let (key, key_span) = cur.expect_ident()?;
            match key.as_str() {
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
    Ok(FormDecl { name: S::new(name, name_span), target, fields, on_submit, on_success, span })
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
    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(PropertyDecl { name, body, span })
}

fn parse_dbtest_decl(cur: &mut Cursor<'_>) -> Result<DbTestDecl, ParseError> {
    let start = cur.expect(&Token::DbTest)?;
    let (tok, _) = cur.bump().ok_or(ParseError { kind: ParseErrorKind::UnexpectedEof, span: start })?;
    let name = if let Token::StringLit(s) = tok { s.to_string() } else { String::new() };
    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(DbTestDecl { name, body, span })
}
