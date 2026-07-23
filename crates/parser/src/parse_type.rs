use certo_ast::span::S;
use certo_ast::types::{TypeExpr, TypeParam, EffectSet, Effect, TraitBound, Bound, RowBound, ModulePath, RecordTypeField};
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};

/// Parse a type expression.
///
/// type = named_type | tuple_type | fn_type | record_type | ptr_type | param
pub fn parse_type(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    let start = cur.peek_span();

    let mut ty = parse_type_atom(cur)?;

    // `T?`  →  Option<T>   `T??`  →  Option<Option<T>>
    loop {
        if let Some((_, q_span)) = cur.eat(|t| matches!(t, Token::Question)) {
            let span = ty.span.to(q_span);
            ty = S::new(TypeExpr::Option { inner: Box::new(ty), span }, span);
        } else if let Some((_, q_span)) = cur.eat(|t| matches!(t, Token::DoubleQuestion)) {
            // `??` lexed as one token — expand as two Option wraps
            let span1 = ty.span.to(q_span);
            ty = S::new(TypeExpr::Option { inner: Box::new(ty), span: span1 }, span1);
            let span2 = span1.to(q_span);
            ty = S::new(TypeExpr::Option { inner: Box::new(ty), span: span2 }, span2);
        } else {
            break;
        }
    }

    // `A => B`  — function type (right-associative)
    if cur.eat(|t| matches!(t, Token::FatArrow)).is_some() {
        let ret = parse_type(cur)?;
        let span = start.to(ret.span);
        ty = S::new(TypeExpr::Fn {
            params: vec![ty],
            ret:    Box::new(ret),
            span,
        }, span);
    }

    Ok(ty)
}

fn parse_type_atom(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    match cur.peek() {
        Some(Token::LParen)  => parse_tuple_type(cur),
        Some(Token::LBrace)  => parse_record_type(cur),
        Some(Token::Star)    => parse_ptr_type(cur),
        Some(Token::Ident(_)) => parse_named_type(cur),
        _ => {
            let span = cur.peek_span();
            let found = cur.peek().map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
            Err(ParseError { kind: ParseErrorKind::Expected { expected: "type".into(), found }, span })
        }
    }
}

/// `(A, B, C)` — tuple; single element with no comma is parenthesised type.
fn parse_tuple_type(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    let start = cur.expect(&Token::LParen)?;
    let mut elements = Vec::new();

    if cur.peek() != Some(&Token::RParen) {
        elements.push(parse_type(cur)?);
        while cur.eat(|t| matches!(t, Token::Comma)).is_some() {
            if cur.peek() == Some(&Token::RParen) { break; }
            elements.push(parse_type(cur)?);
        }
    }

    let end = cur.expect(&Token::RParen)?;
    let span = start.to(end);

    if elements.len() == 1 {
        // Parenthesised type — unwrap
        Ok(elements.remove(0))
    } else {
        Ok(S::new(TypeExpr::Tuple { elements, span }, span))
    }
}

/// `{ name: Type, name?: Type }` — anonymous record type.
fn parse_record_type(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    let start = cur.expect(&Token::LBrace)?;
    let mut fields = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let (name, name_span) = cur.expect_ident()?;
        let optional = cur.eat(|t| matches!(t, Token::Question)).is_some();
        cur.expect(&Token::Colon)?;
        let ty = parse_type(cur)?;
        let span = name_span.to(ty.span);
        fields.push(RecordTypeField {
            name: S::new(name, name_span),
            ty,
            optional,
            span,
        });
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(S::new(TypeExpr::Record { fields, span }, span))
}

/// `*T` — raw pointer (unsafe/FFI)
fn parse_ptr_type(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    let start = cur.expect(&Token::Star)?;
    let inner = parse_type_atom(cur)?;
    let span = start.to(inner.span);
    Ok(S::new(TypeExpr::Ptr { inner: Box::new(inner), span }, span))
}

/// `ModulePath.Name<T, U>`
fn parse_named_type(cur: &mut Cursor<'_>) -> Result<S<TypeExpr>, ParseError> {
    let path = parse_module_path(cur)?;
    let span_start = path.span;

    // Generic args: `<T, U>`
    let args = if cur.peek() == Some(&Token::Lt) {
        cur.bump();
        let mut args = Vec::new();
        args.push(parse_type(cur)?);
        while cur.eat(|t| matches!(t, Token::Comma)).is_some() {
            if cur.peek() == Some(&Token::Gt) { break; }
            args.push(parse_type(cur)?);
        }
        cur.expect(&Token::Gt)?;
        args
    } else {
        Vec::new()
    };

    let span = if args.is_empty() {
        span_start
    } else {
        span_start.to(cur.peek_span())
    };

    Ok(S::new(TypeExpr::Named { path, args, span }, span))
}

// ------------------------------------------------------------------ //
// Shared helpers used by other parse modules
// ------------------------------------------------------------------ //

/// `Stdlib.Collections.List`  or just `List`
pub fn parse_module_path(cur: &mut Cursor<'_>) -> Result<ModulePath, ParseError> {
    let (first, first_span) = cur.expect_ident()?;
    let mut segments = vec![S::new(first, first_span)];
    let mut span = first_span;

    // Look ahead: `Ident.Ident` — but not `Ident.field` (lowercase after dot)
    while cur.peek() == Some(&Token::Dot) {
        if let Some(Token::Ident(next)) = cur.peek2() {
            let starts_upper = next.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
            if !starts_upper { break; }
            cur.bump(); // eat the dot
            let (name, name_span) = cur.expect_ident()?;
            span = span.to(name_span);
            segments.push(S::new(name, name_span));
        } else {
            break;
        }
    }

    Ok(ModulePath { segments, span })
}

/// `<T: Bound, U>` — generic parameter list
pub fn parse_type_params(cur: &mut Cursor<'_>) -> Result<Vec<TypeParam>, ParseError> {
    if cur.peek() != Some(&Token::Lt) {
        return Ok(Vec::new());
    }
    cur.bump(); // eat `<`

    let mut params = Vec::new();
    loop {
        let (name, name_span) = cur.expect_ident()?;
        let mut bounds = Vec::new();

        if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
            bounds.push(parse_bound(cur)?);
            while cur.eat(|t| matches!(t, Token::Plus)).is_some() {
                bounds.push(parse_bound(cur)?);
            }
        }

        let span = name_span; // good enough for now
        params.push(TypeParam { name: S::new(name, name_span), bounds, span });

        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        if cur.peek() == Some(&Token::Gt) { break; }
    }

    cur.expect(&Token::Gt)?;
    Ok(params)
}

fn parse_trait_bound(cur: &mut Cursor<'_>) -> Result<TraitBound, ParseError> {
    let path = parse_module_path(cur)?;
    let span = path.span;
    Ok(TraitBound { name: path, span })
}

/// One bound in a `+`-separated type-param bound list: a trait name (`DbModel`)
/// or an inline record shape (`{ name: Text }`) for row polymorphism.
fn parse_bound(cur: &mut Cursor<'_>) -> Result<Bound, ParseError> {
    if cur.peek() == Some(&Token::LBrace) {
        let rec = parse_record_type(cur)?;
        let TypeExpr::Record { fields, span } = rec.node else {
            unreachable!("parse_record_type always returns TypeExpr::Record")
        };
        Ok(Bound::Row(RowBound { fields, span }))
    } else {
        Ok(Bound::Trait(parse_trait_bound(cur)?))
    }
}

/// `[pure]` or `[db.read, async]`
pub fn parse_effect_set(cur: &mut Cursor<'_>) -> Result<Option<EffectSet>, ParseError> {
    if cur.peek() != Some(&Token::LBracket) {
        return Ok(None);
    }
    let start = cur.bump().unwrap().1; // eat `[`

    let mut effects = Vec::new();
    loop {
        let (name, name_span) = cur.expect_ident()?;
        // Handle `db.read` / `db.write`
        let full = if cur.peek() == Some(&Token::Dot) {
            cur.bump();
            let (sub, _) = cur.expect_ident()?;
            format!("{name}.{sub}")
        } else {
            name
        };

        let effect = match full.as_str() {
            "pure"     => Effect::Pure,
            "db.read"  => Effect::DbRead,
            "db.write" => Effect::DbWrite,
            "io"       => Effect::Io,
            "async"    => Effect::Async,
            "fallible" => Effect::Fallible,
            "unsafe"   => Effect::Unsafe,
            other      => return Err(ParseError {
                kind: ParseErrorKind::Custom(format!("unknown effect `{other}`")),
                span: name_span,
            }),
        };
        effects.push(S::new(effect, name_span));

        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        if cur.peek() == Some(&Token::RBracket) { break; }
    }

    let end = cur.expect(&Token::RBracket)?;
    let span = start.to(end);
    Ok(Some(EffectSet { effects, span }))
}
