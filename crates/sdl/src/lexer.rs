//! Hand-written SDL lexer. Words are all lexed as `Ident`; the parser treats
//! keywords contextually. `//` starts a line comment.

use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;

#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    Ident(String),
    Str(String),
    Num(u64),
    /// A number with a fractional part, kept as text so no precision is lost.
    Dec(String),
    LBrace, RBrace, LParen, RParen,
    Colon, Comma, Arrow, Dot, Eq,
    EqEq, NotEq, Lt, Le, Gt, Ge,
    Plus, Minus, Star, Slash,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

/// Lex `src`. Always ends with an `Eof` token; bad input yields diagnostics
/// and is skipped so the parser still sees a usable stream.
pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut diags = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => { i += 1; }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' { i += 1; }
            }
            b'A'..=b'Z' | b'a'..=b'z' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') { i += 1; }
                push(&mut toks, TokKind::Ident(src[start..i].to_string()), start, i);
            }
            b'0'..=b'9' => {
                while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                    push(&mut toks, TokKind::Dec(src[start..i].to_string()), start, i);
                    continue;
                }
                match src[start..i].parse::<u64>() {
                    Ok(n) => push(&mut toks, TokKind::Num(n), start, i),
                    Err(_) => diags.push(
                        Diagnostic::error("SDL002", "number literal is too large")
                            .with_span(Span::new(start, i)),
                    ),
                }
            }
            b'"' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < b.len() {
                    match b[i] {
                        b'"' => { i += 1; closed = true; break; }
                        b'\\' if i + 1 < b.len() => {
                            s.push(match b[i + 1] { b'n' => '\n', b't' => '\t', o => o as char });
                            i += 2;
                        }
                        _ => {
                            let ch = src[i..].chars().next().unwrap();
                            s.push(ch);
                            i += ch.len_utf8();
                        }
                    }
                }
                if closed {
                    push(&mut toks, TokKind::Str(s), start, i);
                } else {
                    diags.push(
                        Diagnostic::error("SDL003", "unterminated string literal")
                            .with_span(Span::new(start, i)),
                    );
                }
            }
            _ => {
                let two = |k: u8| b.get(i + 1) == Some(&k);
                let (kind, len) = match c {
                    b'{' => (Some(TokKind::LBrace), 1),
                    b'}' => (Some(TokKind::RBrace), 1),
                    b'(' => (Some(TokKind::LParen), 1),
                    b')' => (Some(TokKind::RParen), 1),
                    b':' => (Some(TokKind::Colon), 1),
                    b',' => (Some(TokKind::Comma), 1),
                    b'.' => (Some(TokKind::Dot), 1),
                    b'+' => (Some(TokKind::Plus), 1),
                    b'*' => (Some(TokKind::Star), 1),
                    b'/' => (Some(TokKind::Slash), 1),
                    b'-' if two(b'>') => (Some(TokKind::Arrow), 2),
                    b'-' => (Some(TokKind::Minus), 1),
                    b'=' if two(b'=') => (Some(TokKind::EqEq), 2),
                    b'=' => (Some(TokKind::Eq), 1),
                    b'!' if two(b'=') => (Some(TokKind::NotEq), 2),
                    b'<' if two(b'=') => (Some(TokKind::Le), 2),
                    b'<' => (Some(TokKind::Lt), 1),
                    b'>' if two(b'=') => (Some(TokKind::Ge), 2),
                    b'>' => (Some(TokKind::Gt), 1),
                    _ => (None, 0),
                };
                match kind {
                    Some(k) => { push(&mut toks, k, start, start + len); i += len; }
                    None => {
                        let ch = src[i..].chars().next().unwrap();
                        diags.push(
                            Diagnostic::error("SDL001", format!("unexpected character `{ch}`"))
                                .with_span(Span::new(start, start + ch.len_utf8())),
                        );
                        i += ch.len_utf8();
                    }
                }
            }
        }
    }
    push(&mut toks, TokKind::Eof, b.len(), b.len());
    (toks, diags)
}

fn push(toks: &mut Vec<Token>, kind: TokKind, start: usize, end: usize) {
    toks.push(Token { kind, span: Span::new(start, end) });
}
