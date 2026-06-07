use certo_ast::span::{S, Span};
use certo_ast::pattern::{Pattern, PatternField, LitPat};
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};


/// Parse a pattern. Handles or-patterns (`p1 | p2`) at the top level.
pub fn parse_pattern(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let left = parse_pattern_atom(cur)?;

    if cur.peek() == Some(&Token::Bar) {
        cur.bump();
        let right = parse_pattern(cur)?;
        let span = left.span.to(right.span);
        return Ok(S::new(Pattern::Or {
            left: Box::new(left),
            right: Box::new(right),
            span,
        }, span));
    }

    Ok(left)
}

fn parse_pattern_atom(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let span = cur.peek_span();

    match cur.peek() {
        // Wildcard `_`
        Some(Token::Ident(s)) if *s == "_" => {
            cur.bump();
            Ok(S::new(Pattern::Wildcard { span }, span))
        }

        // Constructor or plain ident
        Some(Token::Ident(_)) => parse_ident_or_constructor(cur),

        // Tuple pattern `(p, q)`
        Some(Token::LParen) => parse_tuple_pattern(cur),

        // List pattern `[head, ...tail]`
        Some(Token::LBracket) => parse_list_pattern(cur),

        // Literals
        Some(Token::Integer(_))
        | Some(Token::Float(_))
        | Some(Token::StringLit(_))
        | Some(Token::True)
        | Some(Token::False)
        | Some(Token::Unit) => parse_literal_pattern(cur),

        // Negative integer / float
        Some(Token::Minus) => {
            cur.bump();
            parse_negative_literal(cur, span)
        }

        _ => {
            let found = cur.peek().map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
            Err(ParseError {
                kind: ParseErrorKind::Expected { expected: "pattern".into(), found },
                span,
            })
        }
    }
}

/// `Name` / `Name(fields)` / `Module.Name(fields)` / `Name { field: pat }`
fn parse_ident_or_constructor(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let (name, name_span) = cur.expect_ident()?;

    // Could be a qualified path like `Ok` or `OrderStatus.Active`
    // Re-use by building a minimal ModulePath then checking for constructor payload
    let mut segments = vec![S::new(name.clone(), name_span)];
    let mut path_span = name_span;

    while cur.peek() == Some(&Token::Dot) {
        if let Some(Token::Ident(next)) = cur.peek2() {
            if next.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                cur.bump(); // eat dot
                let (seg, seg_span) = cur.expect_ident()?;
                path_span = path_span.to(seg_span);
                segments.push(S::new(seg, seg_span));
                continue;
            }
        }
        break;
    }

    let path = certo_ast::types::ModulePath { segments, span: path_span };

    // Is it a constructor with positional fields?  `Circle(r, ...)`
    if cur.peek() == Some(&Token::LParen) {
        cur.bump();
        let mut fields = Vec::new();
        while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
            fields.push(parse_pattern(cur)?);
            if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        }
        let end = cur.expect(&Token::RParen)?;
        let span = path_span.to(end);
        return Ok(S::new(Pattern::Constructor { path, fields, span }, span));
    }

    // Record pattern  `User { name, email: e }`
    if cur.peek() == Some(&Token::LBrace) {
        return parse_record_pattern(cur, Some(path), path_span);
    }

    // Plain ident — but only bind if it starts lowercase (conventional)
    // Uppercase with no payload = unit constructor
    let first_char = name.chars().next().unwrap_or('_');
    if first_char.is_uppercase() {
        return Ok(S::new(Pattern::Constructor { path, fields: vec![], span: path_span }, path_span));
    }

    // Lowercase = binding
    let ident_node = path.segments.into_iter().next().unwrap();
    Ok(S::new(Pattern::Ident { name: ident_node, span: path_span }, path_span))
}

/// `{ name, email: e, .. }`
fn parse_record_pattern(
    cur:        &mut Cursor<'_>,
    path:       Option<certo_ast::types::ModulePath>,
    start_span: Span,
) -> Result<S<Pattern>, ParseError> {
    cur.bump(); // eat `{`
    let mut fields = Vec::new();
    let mut rest = false;

    while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
        if cur.peek() == Some(&Token::DotDot) {
            cur.bump();
            rest = true;
            break;
        }
        let (fname, fspan) = cur.expect_ident()?;
        let pattern = if cur.eat(|t| matches!(t, Token::Colon)).is_some() {
            Some(parse_pattern(cur)?)
        } else {
            None
        };
        fields.push(PatternField { name: S::new(fname, fspan), pattern, span: fspan });
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RBrace)?;
    let span = start_span.to(end);
    Ok(S::new(Pattern::Record { path, fields, rest, span }, span))
}

/// `(p, q, r)`
fn parse_tuple_pattern(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let start = cur.expect(&Token::LParen)?;
    let mut elements = Vec::new();

    while cur.peek() != Some(&Token::RParen) && !cur.at_end() {
        elements.push(parse_pattern(cur)?);
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RParen)?;
    let span = start.to(end);

    if elements.len() == 1 {
        Ok(elements.remove(0))
    } else {
        Ok(S::new(Pattern::Tuple { elements, span }, span))
    }
}

/// `[head, second, ...tail]`
fn parse_list_pattern(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let start = cur.expect(&Token::LBracket)?;
    let mut head = Vec::new();
    let mut tail = None;

    while cur.peek() != Some(&Token::RBracket) && !cur.at_end() {
        if cur.peek() == Some(&Token::DotDotDot) {
            cur.bump();
            let rest = parse_pattern(cur)?;
            tail = Some(Box::new(rest));
            break;
        }
        head.push(parse_pattern(cur)?);
        if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
    }

    let end = cur.expect(&Token::RBracket)?;
    let span = start.to(end);
    Ok(S::new(Pattern::List { head, tail, span }, span))
}

fn parse_literal_pattern(cur: &mut Cursor<'_>) -> Result<S<Pattern>, ParseError> {
    let (tok, span) = cur.bump().unwrap();
    let lit = match tok {
        Token::Integer(s) => {
            let n = s.parse::<i64>().map_err(|e| ParseError {
                kind: ParseErrorKind::InvalidLiteral(e.to_string()),
                span,
            })?;
            LitPat::Int(n)
        }
        Token::Float(s) => {
            let f = s.parse::<f64>().map_err(|e| ParseError {
                kind: ParseErrorKind::InvalidLiteral(e.to_string()),
                span,
            })?;
            LitPat::Float(f)
        }
        Token::StringLit(s) => LitPat::String(s.to_string()),
        Token::True  => LitPat::Bool(true),
        Token::False => LitPat::Bool(false),
        Token::Unit  => LitPat::Unit,
        _ => unreachable!(),
    };
    Ok(S::new(Pattern::Literal { value: lit, span }, span))
}

fn parse_negative_literal(cur: &mut Cursor<'_>, minus_span: Span) -> Result<S<Pattern>, ParseError> {
    let (tok, tok_span) = cur.bump().ok_or(ParseError {
        kind: ParseErrorKind::UnexpectedEof,
        span: minus_span,
    })?;
    let span = minus_span.to(tok_span);
    let lit = match tok {
        Token::Integer(s) => {
            let n = s.parse::<i64>().map_err(|e| ParseError {
                kind: ParseErrorKind::InvalidLiteral(e.to_string()),
                span,
            })?;
            LitPat::Int(-n)
        }
        Token::Float(s) => {
            let f = s.parse::<f64>().map_err(|e| ParseError {
                kind: ParseErrorKind::InvalidLiteral(e.to_string()),
                span,
            })?;
            LitPat::Float(-f)
        }
        _ => return Err(ParseError {
            kind: ParseErrorKind::Expected { expected: "integer or float after `-`".into(), found: format!("{tok:?}") },
            span,
        }),
    };
    Ok(S::new(Pattern::Literal { value: lit, span }, span))
}
