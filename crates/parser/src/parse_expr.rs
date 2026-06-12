use certo_ast::span::{S, Span};
use certo_ast::expr::*;

use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};
use crate::parse_type::{parse_type, parse_module_path};
use crate::parse_pattern::parse_pattern;

// ------------------------------------------------------------------ //
// Entry point — parse a full expression
// ------------------------------------------------------------------ //

pub fn parse_expr(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    parse_pipeline(cur)
}

// ------------------------------------------------------------------ //
// Precedence levels (low → high)
//
//   pipeline     |>
//   assign       (statements only; handled in parse_block)
//   or           or
//   and          and
//   not          not
//   comparison   == != < > <= >=
//   range        .. ...
//   add/sub      + -
//   mul/div      * / %
//   power        **   (right-associative)
//   unary        - not
//   postfix      . ?. () ? await
//   atom
// ------------------------------------------------------------------ //

fn parse_pipeline(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_or(cur)?;
    while cur.peek() == Some(&Token::Pipe) {
        cur.bump();
        let right = parse_or(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::Pipe { left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_or(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_and(cur)?;
    while cur.peek() == Some(&Token::Or) {
        cur.bump();
        let right = parse_and(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_and(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_coalesce(cur)?;
    while cur.peek() == Some(&Token::And) {
        cur.bump();
        let right = parse_not(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op: BinOp::And, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_coalesce(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_not(cur)?;
    while cur.peek() == Some(&Token::DoubleQuestion) {
        cur.bump();
        let right = parse_not(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op: BinOp::NullCoalesce, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_not(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    if let Some((_, span)) = cur.eat(|t| matches!(t, Token::Not | Token::Bang)) {
        let expr = parse_not(cur)?;
        let full_span = span.to(expr.span);
        return Ok(S::new(Expr::UnOp { op: UnOp::Not, expr: Box::new(expr), span: full_span }, full_span));
    }
    parse_comparison(cur)
}

fn parse_comparison(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_range(cur)?;
    loop {
        let op = match cur.peek() {
            Some(Token::EqEq)  => BinOp::Eq,
            Some(Token::NotEq) => BinOp::NotEq,
            Some(Token::Lt)    => BinOp::Lt,
            Some(Token::LtEq)  => BinOp::LtEq,
            Some(Token::Gt)    => BinOp::Gt,
            Some(Token::GtEq)  => BinOp::GtEq,
            _                  => break,
        };
        cur.bump();
        let right = parse_range(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_range(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let left = parse_add(cur)?;
    let op = match cur.peek() {
        Some(Token::DotDotDot) => BinOp::RangeExclusive,
        Some(Token::DotDot)    => BinOp::RangeInclusive,
        _                      => return Ok(left),
    };
    cur.bump();
    let right = parse_add(cur)?;
    let span = left.span.to(right.span);
    Ok(S::new(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span }, span))
}

fn parse_add(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_mul(cur)?;
    loop {
        let op = match cur.peek() {
            Some(Token::Plus)     => BinOp::Add,
            Some(Token::Minus)    => BinOp::Sub,
            Some(Token::PlusPlus) => BinOp::Concat,
            _                     => break,
        };
        cur.bump();
        let right = parse_mul(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_mul(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut left = parse_power(cur)?;
    loop {
        let op = match cur.peek() {
            Some(Token::Star)    => BinOp::Mul,
            Some(Token::Slash)   => BinOp::Div,
            Some(Token::Percent) => BinOp::Rem,
            _                    => break,
        };
        cur.bump();
        let right = parse_power(cur)?;
        let span = left.span.to(right.span);
        left = S::new(Expr::BinOp { op, left: Box::new(left), right: Box::new(right), span }, span);
    }
    Ok(left)
}

fn parse_power(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let base = parse_unary(cur)?;
    if cur.peek() == Some(&Token::StarStar) {
        cur.bump();
        let exp = parse_power(cur)?; // right-associative
        let span = base.span.to(exp.span);
        return Ok(S::new(Expr::BinOp { op: BinOp::Pow, left: Box::new(base), right: Box::new(exp), span }, span));
    }
    Ok(base)
}

fn parse_unary(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    if let Some((_, span)) = cur.eat(|t| matches!(t, Token::Minus)) {
        let expr = parse_unary(cur)?;
        let full = span.to(expr.span);
        return Ok(S::new(Expr::UnOp { op: UnOp::Neg, expr: Box::new(expr), span: full }, full));
    }
    parse_postfix(cur)
}

fn parse_postfix(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let mut expr = parse_atom(cur)?;

    loop {
        match cur.peek() {
            // `expr.field`
            Some(Token::Dot) => {
                cur.bump();
                let (field, field_span) = cur.expect_ident()?;
                let span = expr.span.to(field_span);
                expr = S::new(Expr::Field {
                    expr: Box::new(expr),
                    field: S::new(field, field_span),
                    span,
                }, span);
            }
            // `expr?.field`
            Some(Token::SafeDot) => {
                cur.bump();
                let (field, field_span) = cur.expect_ident()?;
                let span = expr.span.to(field_span);
                expr = S::new(Expr::SafeField {
                    expr: Box::new(expr),
                    field: S::new(field, field_span),
                    span,
                }, span);
            }
            // `expr(args)`
            Some(Token::LParen) => {
                let (args, end_span) = parse_arg_list(cur)?;
                let span = expr.span.to(end_span);
                expr = S::new(Expr::App { func: Box::new(expr), args, span }, span);
            }
            // `expr?`
            Some(Token::Question) => {
                let (_, q_span) = cur.bump().unwrap();
                let span = expr.span.to(q_span);
                expr = S::new(Expr::Try { expr: Box::new(expr), span }, span);
            }
            _ => break,
        }
    }

    Ok(expr)
}

fn parse_arg_list(cur: &mut Cursor<'_>) -> Result<(Vec<Arg>, Span), ParseError> {
    cur.expect(&Token::LParen)?;
    let mut args = Vec::new();

    while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
        // Named arg: `name: expr`
        let label = if let Some(Token::Ident(_)) = cur.peek() {
            if cur.peek2() == Some(&Token::Colon) {
                let (name, name_span) = cur.expect_ident()?;
                cur.bump(); // eat `:`
                Some(S::new(name, name_span))
            } else {
                None
            }
        } else {
            None
        };

        let value = parse_expr(cur)?;
        let span = label.as_ref().map(|l| l.span).unwrap_or(value.span).to(value.span);
        args.push(Arg { label, value, span });

        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RParen)?;
    Ok((args, end))
}

// ------------------------------------------------------------------ //
// Atoms
// ------------------------------------------------------------------ //

fn parse_atom(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let span = cur.peek_span();

    match cur.peek() {
        // Literals
        Some(Token::Integer(_))
        | Some(Token::Float(_))
        | Some(Token::Decimal(_))
        | Some(Token::StringLit(_))
        | Some(Token::FString(_))
        | Some(Token::UuidLit(_))
        | Some(Token::MultilineString(_))
        | Some(Token::True)
        | Some(Token::False)
        | Some(Token::Unit)   => parse_literal(cur),

        // Identifier / path / constructor
        Some(Token::Ident(_)) => parse_ident_or_record(cur),

        // `(expr)` or tuple
        Some(Token::LParen)   => parse_paren_or_tuple(cur),

        // `[e1, e2]` — list
        Some(Token::LBracket) => parse_list(cur),

        // `{ ... }` — block or record
        Some(Token::LBrace)   => parse_block_or_record(cur),

        // `if cond then a else b`
        Some(Token::If)       => parse_if(cur),

        // `match expr { ... }`
        Some(Token::Match)    => parse_match(cur),

        // `await expr`
        Some(Token::Await)    => {
            cur.bump();
            let expr = parse_postfix(cur)?;
            let full = span.to(expr.span);
            Ok(S::new(Expr::Await { expr: Box::new(expr), span: full }, full))
        }

        // `spawn expr`
        Some(Token::Spawn)    => {
            cur.bump();
            let expr = parse_postfix(cur)?;
            let full = span.to(expr.span);
            Ok(S::new(Expr::Spawn { expr: Box::new(expr), span: full }, full))
        }

        // `guard cond else expr`
        Some(Token::Guard)    => parse_guard(cur),

        // `require expr (Error)`
        Some(Token::Require)  => parse_require(cur),

        // `parallel { ... }`
        Some(Token::Parallel) => parse_parallel(cur),

        // `for x in iter { body }`
        Some(Token::For) => parse_for(cur),

        // `while cond { body }`
        Some(Token::While) => parse_while(cur),

        _ => {
            let found = cur.peek().map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
            Err(ParseError { kind: ParseErrorKind::Expected { expected: "expression".into(), found }, span })
        }
    }
}

fn parse_literal(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let (tok, span) = cur.bump().unwrap();
    let lit = match tok {
        Token::Integer(s)       => {
            // Hex / binary / octal need special parsing
            let n = if s.starts_with("0x") || s.starts_with("0X") {
                i64::from_str_radix(&s[2..].replace('_', ""), 16)
            } else if s.starts_with("0b") || s.starts_with("0B") {
                i64::from_str_radix(&s[2..].replace('_', ""), 2)
            } else if s.starts_with("0o") || s.starts_with("0O") {
                i64::from_str_radix(&s[2..].replace('_', ""), 8)
            } else {
                s.replace('_', "").parse::<i64>()
            };
            Lit::Int(n.map_err(|e| ParseError { kind: ParseErrorKind::InvalidLiteral(e.to_string()), span })?)
        }
        Token::Float(s)         => {
            let f = s.replace('_', "").parse::<f64>().map_err(|e| ParseError {
                kind: ParseErrorKind::InvalidLiteral(e.to_string()), span,
            })?;
            Lit::Float(f)
        }
        Token::Decimal(s)       => Lit::Decimal(s.to_string()),
        Token::StringLit(s)     => Lit::String(unescape_str(s)),
        Token::MultilineString(s) => Lit::String(unescape_str(s)),
        Token::FString(s)       => Lit::FString(parse_fstring_parts(s, span)?),
        Token::UuidLit(s)       => Lit::Uuid(s.to_string()),
        Token::True             => Lit::Bool(true),
        Token::False            => Lit::Bool(false),
        Token::Unit             => Lit::Unit,
        _ => unreachable!(),
    };
    Ok(S::new(Expr::Lit { value: lit, span }, span))
}

fn parse_ident_or_record(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let path = parse_module_path(cur)?;
    let span = path.span;

    // `Type { field: val }` — record literal.
    // Only treat as a record when the leading name is uppercase (a type constructor),
    // never for lowercase identifiers like the scrutinee in `match y { ... }`.
    let first_is_upper = path.segments.first()
        .map(|s| s.node.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
        .unwrap_or(false);

    if first_is_upper && cur.peek() == Some(&Token::LBrace) {
        let name = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
        return parse_record_body(cur, Some(name), None, span);
    }

    Ok(S::new(Expr::Path { path, span }, span))
}

fn parse_paren_or_tuple(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.expect(&Token::LParen)?;
    if cur.peek() == Some(&Token::RParen) {
        let end = cur.bump().unwrap().1;
        let span = start.to(end);
        return Ok(S::new(Expr::Lit { value: Lit::Unit, span }, span));
    }

    let first = parse_expr(cur)?;
    if cur.eat(|t| matches!(t, Token::Comma)).is_some() {
        let mut elements = vec![first];
        while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
            elements.push(parse_expr(cur)?);
            if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        }
        let end = cur.expect(&Token::RParen)?;
        let span = start.to(end);
        return Ok(S::new(Expr::Tuple { elements, span }, span));
    }

    cur.expect(&Token::RParen)?;
    Ok(first)
}

fn parse_list(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.expect(&Token::LBracket)?;
    let mut elements = Vec::new();

    while cur.peek() != Some(&Token::RBracket) && !cur.at_end() {
        elements.push(parse_expr(cur)?);
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RBracket)?;
    let span = start.to(end);
    Ok(S::new(Expr::List { elements, span }, span))
}

/// `{ stmts... }` — block, OR `{ field: val, ... }` — record literal.
///
/// Disambiguate by peeking: `{ ident: expr` → record; everything else → block.
fn parse_block_or_record(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    parse_block(cur)
}

pub fn parse_block(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.expect(&Token::LBrace)?;
    let mut stmts = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let stmt = parse_stmt(cur)?;
        stmts.push(stmt);
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(S::new(Expr::Block { stmts, span }, span))
}

fn parse_stmt(cur: &mut Cursor<'_>) -> Result<Stmt, ParseError> {
    let span = cur.peek_span();

    match cur.peek() {
        Some(Token::Val) => {
            cur.bump();
            let pattern = parse_pattern(cur)?;
            let ty = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
                Some(parse_type(cur)?)
            } else {
                None
            };
            cur.expect(&Token::Eq)?;
            let value = parse_expr(cur)?;
            let span = span.to(value.span);
            Ok(Stmt::Val { pattern, ty, value, span })
        }
        Some(Token::Var) => {
            cur.bump();
            let (name, name_span) = cur.expect_ident()?;
            let ty = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
                Some(parse_type(cur)?)
            } else {
                None
            };
            cur.expect(&Token::Eq)?;
            let value = parse_expr(cur)?;
            let span = span.to(value.span);
            Ok(Stmt::Var { name: S::new(name, name_span), ty, value, span })
        }
        Some(Token::Defer) => {
            cur.bump();
            let body = parse_block(cur)?;
            let span = span.to(body.span);
            Ok(Stmt::Defer { body, span })
        }
        _ => {
            // Could be `name = expr` (assignment) or bare expression
            let expr = parse_expr(cur)?;

            // Check for assignment: `ident =` (but not `==`)
            if let Expr::Path { path, .. } = &expr.node {
                if path.segments.len() == 1 && cur.peek() == Some(&Token::Eq) {
                    cur.bump();
                    let value = parse_expr(cur)?;
                    let name = path.segments[0].node.clone();
                    let name_span = path.segments[0].span;
                    let span = span.to(value.span);
                    return Ok(Stmt::Assign { target: S::new(name, name_span), value, span });
                }
            }

            let span = expr.span;
            Ok(Stmt::Expr { expr, span })
        }
    }
}

fn parse_record_body(
    cur:     &mut Cursor<'_>,
    ty_name: Option<String>,
    base:    Option<S<Expr>>,
    start:   Span,
) -> Result<S<Expr>, ParseError> {
    cur.expect(&Token::LBrace)?;
    let mut fields = Vec::new();

    // `..base_expr` spread — copies all fields from base_expr, overriding those listed below.
    let spread_base = if cur.eat(|t| matches!(t, Token::DotDot)).is_some() {
        let b = parse_expr(cur)?;
        let _ = cur.eat(|t| matches!(t, Token::Comma));
        Some(b)
    } else {
        base
    };

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let (name, name_span) = cur.expect_ident()?;
        cur.expect(&Token::Colon)?;
        let value = parse_expr(cur)?;
        let span = name_span.to(value.span);
        fields.push(RecordField { name: S::new(name, name_span), value, span });
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(S::new(Expr::Record { ty_name, base: spread_base.map(Box::new), fields, span }, span))
}

fn parse_if(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.peek_span();
    cur.bump(); // eat `if`

    // `if let Pat = expr { body } [else { other }]`
    // Desugar to: match expr { Pat => body, _ => other_or_unit }
    if cur.eat(|t| matches!(t, Token::Let)).is_some() {
        let pat = parse_pattern(cur)?;
        cur.expect(&Token::Eq)?;
        let scrutinee = parse_expr(cur)?;
        let body = parse_block(cur)?;
        let else_expr = if cur.eat(|t| matches!(t, Token::Else)).is_some() {
            parse_block(cur)?
        } else {
            let sp = body.span;
            S::new(Expr::Lit { value: Lit::Unit, span: sp }, sp)
        };
        let span = start.to(else_expr.span);
        let wildcard_span = else_expr.span;
        let match_arm = MatchArm {
            pattern: pat,
            guard:   None,
            body,
            span:    span,
        };
        let wildcard_arm = MatchArm {
            pattern: S::new(certo_ast::pattern::Pattern::Wildcard { span: wildcard_span }, wildcard_span),
            guard:   None,
            body:    else_expr,
            span:    wildcard_span,
        };
        return Ok(S::new(Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms:      vec![match_arm, wildcard_arm],
            span,
        }, span));
    }

    let cond = parse_expr(cur)?;
    cur.expect(&Token::Then)?;
    let then_expr = parse_expr(cur)?;
    let (else_expr, span) = if cur.eat(|t| matches!(t, Token::Else)).is_some() {
        let e = parse_expr(cur)?;
        let sp = start.to(e.span);
        (e, sp)
    } else {
        // Synthesise a Unit else-branch so `if cond then stmt` is valid.
        let sp = start.to(then_expr.span);
        (S::new(Expr::Block { stmts: vec![], span: sp }, sp), sp)
    };
    Ok(S::new(Expr::If {
        cond: Box::new(cond),
        then_expr: Box::new(then_expr),
        else_expr: Box::new(else_expr),
        span,
    }, span))
}

fn parse_match(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.peek_span();
    cur.bump(); // eat `match`
    let scrutinee = parse_expr(cur)?;
    cur.expect(&Token::LBrace)?;
    let mut arms = Vec::new();

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        let pattern = parse_pattern(cur)?;
        let guard = if cur.eat(|t| matches!(t, Token::If)).is_some() {
            Some(parse_expr(cur)?)
        } else {
            None
        };
        cur.expect(&Token::FatArrow)?;
        let body = parse_expr(cur)?;
        let span = pattern.span.to(body.span);
        arms.push(MatchArm { pattern, guard, body, span });
        // Optional trailing newline / comma between arms
        cur.eat(|t| matches!(t, Token::Comma));
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(S::new(Expr::Match { scrutinee: Box::new(scrutinee), arms, span }, span))
}

fn parse_guard(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.peek_span();
    cur.bump(); // eat `guard`
    let cond = parse_expr(cur)?;
    cur.expect(&Token::Else)?;
    let else_expr = parse_expr(cur)?;
    let span = start.to(else_expr.span);
    Ok(S::new(Expr::Guard {
        cond: Box::new(cond),
        else_expr: Box::new(else_expr),
        span,
    }, span))
}

fn parse_require(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.peek_span();
    cur.bump(); // eat `require`
    let expr = parse_postfix(cur)?;
    cur.expect(&Token::LParen)?;
    let error = parse_expr(cur)?;
    let end = cur.expect(&Token::RParen)?;
    let span = start.to(end);
    Ok(S::new(Expr::Require {
        expr: Box::new(expr),
        error: Box::new(error),
        span,
    }, span))
}

fn parse_parallel(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.peek_span();
    cur.bump(); // eat `parallel`

    // Optional `(timeout: Duration.seconds(5))`
    let timeout = if cur.peek() == Some(&Token::LParen) {
        cur.bump();
        // expect `timeout: expr`
        let (kw, kw_span) = cur.expect_ident()?;
        if kw != "timeout" {
            return Err(ParseError {
                kind: ParseErrorKind::Custom("expected `timeout` keyword".into()),
                span: kw_span,
            });
        }
        cur.expect(&Token::Colon)?;
        let t = parse_expr(cur)?;
        cur.expect(&Token::RParen)?;
        Some(Box::new(t))
    } else {
        None
    };

    cur.expect(&Token::LBrace)?;
    let mut tasks = Vec::new();
    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        tasks.push(parse_expr(cur)?);
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }
    let end = cur.expect(&Token::RBrace)?;
    let span = start.to(end);
    Ok(S::new(Expr::Parallel { tasks, timeout, span }, span))
}

/// Split an f-string content string into literal and interpolated parts.
/// e.g. `"Hello, {name}! You have {count} items."` →
///   [Literal("Hello, "), Interp(name), Literal("! You have "), Interp(count), Literal(" items.")]
fn parse_fstring_parts(content: &str, base_span: Span) -> Result<Vec<FStringPart>, ParseError> {
    use crate::cursor::Cursor as C;
    let mut parts = Vec::new();
    let mut rest = content;
    while !rest.is_empty() {
        if let Some(open) = rest.find('{') {
            if open > 0 {
                parts.push(FStringPart::Literal(rest[..open].to_string()));
            }
            rest = &rest[open + 1..];
            // Find matching `}`, respecting nested braces
            let mut depth = 1usize;
            let mut end = 0;
            for (i, c) in rest.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 { end = i; break; }
                    }
                    _ => {}
                }
            }
            if depth != 0 {
                return Err(ParseError {
                    kind: ParseErrorKind::Custom("unclosed `{` in f-string".into()),
                    span: base_span,
                });
            }
            let inner = &rest[..end];
            rest = &rest[end + 1..];
            // Re-lex and parse the embedded expression
            let tokens = certo_lexer::lex(inner).map_err(|_| ParseError {
                kind: ParseErrorKind::Custom(format!("invalid token in f-string expression `{{{inner}}}`")),
                span: base_span,
            })?;
            let mut sub = C::new(tokens, inner);
            let expr = parse_expr(&mut sub)?;
            parts.push(FStringPart::Interpolated(Box::new(expr)));
        } else {
            parts.push(FStringPart::Literal(rest.to_string()));
            break;
        }
    }
    if parts.is_empty() {
        parts.push(FStringPart::Literal(String::new()));
    }
    Ok(parts)
}

fn parse_while(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.expect(&Token::While)?;
    let cond  = parse_expr(cur)?;
    let body  = parse_block(cur)?;
    let span  = start.to(body.span);
    Ok(S::new(Expr::While { cond: Box::new(cond), body: Box::new(body), span }, span))
}

fn unescape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n')  => out.push('\n'),
                Some('t')  => out.push('\t'),
                Some('r')  => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"')  => out.push('"'),
                Some('0')  => out.push('\0'),
                Some(c)    => { out.push('\\'); out.push(c); }
                None       => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_for(cur: &mut Cursor<'_>) -> Result<S<Expr>, ParseError> {
    let start = cur.expect(&Token::For)?;
    let (binding, binding_span) = cur.expect_ident()?;
    cur.expect(&Token::In)?;
    let iter = parse_expr(cur)?;
    let body = parse_block(cur)?;
    let span = start.to(body.span);
    Ok(S::new(Expr::For {
        binding: S::new(binding, binding_span),
        iter:    Box::new(iter),
        body:    Box::new(body),
        span,
    }, span))
}
