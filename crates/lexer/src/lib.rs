use logos::Logos;

/// All tokens in the Certo language.
///
/// String slices borrow from the source — no allocation during lexing.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")] // whitespace
#[logos(skip r"//[^\n]*")]   // line comments
pub enum Token<'src> {
    // ------------------------------------------------------------------ //
    // Literals
    // ------------------------------------------------------------------ //

    /// Float: digits with a dot or an exponent — must come before Integer
    /// to win when both could match a plain decimal run.
    #[regex(r"[0-9][0-9_]*\.[0-9][0-9_]*([eE][+\-]?[0-9]+)?|[0-9][0-9_]*[eE][+\-]?[0-9]+", priority = 3, callback = |lex| lex.slice())]
    Float(&'src str),

    /// Integer: decimal, hex (0x…), binary (0b…), octal (0o…)
    #[regex(r"0x[0-9a-fA-F_]+|0b[01_]+|0o[0-7_]+|[0-9][0-9_]*", priority = 2, callback = |lex| lex.slice())]
    Integer(&'src str),

    /// Exact decimal: d"19.99"
    #[regex(r#"d"[^"]*""#, |lex| {
        let s = lex.slice();
        &s[2..s.len()-1]   // strip d" and "
    })]
    Decimal(&'src str),

    /// Interpolated string: f"…"  (contents lexed as a raw slice; interpolation is a parser concern)
    #[regex(r#"f"([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        &s[2..s.len()-1]
    })]
    FString(&'src str),

    /// UUID literal: uuid"550e8400-…"
    #[regex(r#"uuid"[^"]*""#, |lex| {
        let s = lex.slice();
        &s[5..s.len()-1]
    })]
    UuidLit(&'src str),

    /// Plain string: "…"
    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        &s[1..s.len()-1]
    })]
    StringLit(&'src str),

    /// Triple-quoted multiline string: """…"""
    #[regex(r#""""([^"]|"[^"]|""[^"])*""""#, priority = 3, callback = |lex| {
        let s = lex.slice();
        &s[3..s.len()-3]
    })]
    MultilineString(&'src str),

    // ------------------------------------------------------------------ //
    // Keywords — must appear before Ident so logos picks the right branch
    // ------------------------------------------------------------------ //
    #[token("fn")]          Fn,
    #[token("type")]        Type,
    #[token("let")]         Let,
    #[token("val")]         Val,
    #[token("var")]         Var,
    #[token("match")]       Match,
    #[token("if")]          If,
    #[token("then")]        Then,
    #[token("else")]        Else,
    #[token("when")]        When,
    #[token("async")]       Async,
    #[token("await")]       Await,
    #[token("parallel")]    Parallel,
    #[token("do")]          Do,
    #[token("return")]      Return,
    #[token("import")]      Import,
    #[token("module")]      Module,
    #[token("export")]      Export,
    #[token("pub")]         Pub,
    #[token("priv")]        Priv,
    #[token("trait")]       Trait,
    #[token("impl")]        Impl,
    #[token("for")]         For,
    #[token("while")]       While,
    #[token("where")]       Where,
    #[token("as")]          As,
    #[token("statemachine")] StateMachine,
    #[token("validator")]   Validator,
    #[token("migration")]   Migration,
    #[token("view")]        View,
    #[token("form")]        Form,
    #[token("test")]        Test,
    #[token("property")]    Property,
    #[token("dbTest")]      DbTest,
    #[token("true")]        True,
    #[token("false")]       False,
    #[token("unit")]        Unit,
    #[token("in")]          In,
    #[token("is")]          Is,
    #[token("not")]         Not,
    #[token("and")]         And,
    #[token("or")]          Or,
    #[token("guard")]       Guard,
    #[token("require")]     Require,
    #[token("ensure")]      Ensure,
    #[token("defer")]       Defer,
    #[token("with")]        With,

    // ------------------------------------------------------------------ //
    // Identifiers  (after keywords so keywords take priority)
    // ------------------------------------------------------------------ //
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", priority = 2, callback = |lex| lex.slice())]
    Ident(&'src str),

    // ------------------------------------------------------------------ //
    // Operators — longer tokens first so logos prefers them
    // ------------------------------------------------------------------ //
    #[token("|>")]  Pipe,           // pipeline
    #[token("=>")]  FatArrow,       // match arm / lambda
    #[token("->")]  Arrow,          // function type
    #[token("?.")]  SafeDot,        // safe field access
    #[token("...")]  DotDotDot,     // exclusive range  (before DotDot)
    #[token("..")]   DotDot,        // inclusive range
    #[token("==")]  EqEq,
    #[token("!=")]  NotEq,
    #[token("<=")]  LtEq,
    #[token(">=")]  GtEq,
    #[token("**")]  StarStar,       // exponentiation
    #[token("??")]  DoubleQuestion, // null-coalesce (DSL use)
    #[token("++")]  PlusPlus,       // string concat
    #[token("+")]   Plus,
    #[token("-")]   Minus,
    #[token("*")]   Star,
    #[token("/")]   Slash,
    #[token("%")]   Percent,
    #[token("<")]   Lt,
    #[token(">")]   Gt,
    #[token(".")]   Dot,
    #[token("=")]   Eq,
    #[token("!")]   Bang,
    #[token("|")]   Bar,            // sum type variant separator
    #[token("&")]   Amp,
    #[token("@")]   At,             // annotations
    #[token("?")]   Question,       // optional type suffix / ? propagation

    // ------------------------------------------------------------------ //
    // Delimiters
    // ------------------------------------------------------------------ //
    #[token("(")]  LParen,
    #[token(")")]  RParen,
    #[token("{")]  LBrace,
    #[token("}")]  RBrace,
    #[token("[")]  LBracket,
    #[token("]")]  RBracket,
    #[token(",")]  Comma,
    #[token(":")]  Colon,
    #[token(";")]  Semi,
    #[token("#")]  Hash,
}

/// A token together with its byte-offset span in the source.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<'src> {
    pub token: Token<'src>,
    pub span:  logos::Span,
}

/// Lex a source string and collect all tokens (or errors).
///
/// Returns `Ok(Vec<Spanned>)` when every byte is recognised.
/// Returns `Err(Vec<(logos::Span, &str)>)` listing every unrecognised span.
pub fn lex(source: &str) -> Result<Vec<Spanned<'_>>, Vec<(logos::Span, &str)>> {
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    for (result, span) in Token::lexer(source).spanned() {
        match result {
            Ok(token) => tokens.push(Spanned { token, span }),
            Err(_)    => errors.push((span.clone(), &source[span])),
        }
    }

    if errors.is_empty() {
        Ok(tokens)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(src: &str) -> Vec<Token<'_>> {
        lex(src).unwrap().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn integer_literals() {
        assert_eq!(tok("42"),     vec![Token::Integer("42")]);
        assert_eq!(tok("0xFF"),   vec![Token::Integer("0xFF")]);
        assert_eq!(tok("0b1010"), vec![Token::Integer("0b1010")]);
        assert_eq!(tok("0o77"),   vec![Token::Integer("0o77")]);
    }

    #[test]
    fn decimal_literal() {
        assert_eq!(tok(r#"d"19.99""#), vec![Token::Decimal("19.99")]);
    }

    #[test]
    fn uuid_literal() {
        assert_eq!(
            tok(r#"uuid"550e8400-e29b-41d4-a716-446655440000""#),
            vec![Token::UuidLit("550e8400-e29b-41d4-a716-446655440000")]
        );
    }

    #[test]
    fn fstring_literal() {
        assert_eq!(tok(r#"f"Total: {amount}""#), vec![Token::FString("Total: {amount}")]);
    }

    #[test]
    fn string_literal() {
        assert_eq!(tok(r#""hello""#), vec![Token::StringLit("hello")]);
    }

    #[test]
    fn keywords() {
        assert_eq!(tok("fn"),           vec![Token::Fn]);
        assert_eq!(tok("statemachine"), vec![Token::StateMachine]);
        assert_eq!(tok("dbTest"),       vec![Token::DbTest]);
        assert_eq!(tok("guard"),        vec![Token::Guard]);
    }

    #[test]
    fn ident_not_keyword() {
        // keywords must not be prefixes of identifiers
        assert_eq!(tok("fooBar"),   vec![Token::Ident("fooBar")]);
        assert_eq!(tok("fnHelper"), vec![Token::Ident("fnHelper")]);
    }

    #[test]
    fn operators() {
        assert_eq!(tok("|>"),  vec![Token::Pipe]);
        assert_eq!(tok("=>"),  vec![Token::FatArrow]);
        assert_eq!(tok("?."),  vec![Token::SafeDot]);
        assert_eq!(tok(".."),  vec![Token::DotDot]);
        assert_eq!(tok("..."), vec![Token::DotDotDot]);
        assert_eq!(tok("**"),  vec![Token::StarStar]);
    }

    #[test]
    fn pipe_expression() {
        let tokens = tok("x |> f");
        assert_eq!(tokens, vec![
            Token::Ident("x"),
            Token::Pipe,
            Token::Ident("f"),
        ]);
    }

    #[test]
    fn simple_function() {
        let tokens = tok("fn add(a: Int, b: Int): Int = a + b");
        assert_eq!(tokens, vec![
            Token::Fn,
            Token::Ident("add"),
            Token::LParen,
            Token::Ident("a"),
            Token::Colon,
            Token::Ident("Int"),
            Token::Comma,
            Token::Ident("b"),
            Token::Colon,
            Token::Ident("Int"),
            Token::RParen,
            Token::Colon,
            Token::Ident("Int"),
            Token::Eq,
            Token::Ident("a"),
            Token::Plus,
            Token::Ident("b"),
        ]);
    }

    #[test]
    fn line_comment_skipped() {
        assert_eq!(tok("// this is a comment\nfn"), vec![Token::Fn]);
    }

    #[test]
    fn optional_type_suffix() {
        let tokens = tok("Timestamp?");
        assert_eq!(tokens, vec![Token::Ident("Timestamp"), Token::Question]);
    }

    #[test]
    fn sum_type_bar() {
        // | separating sum type variants
        let tokens = tok("| Pending | Active");
        assert_eq!(tokens, vec![
            Token::Bar,
            Token::Ident("Pending"),
            Token::Bar,
            Token::Ident("Active"),
        ]);
    }

    #[test]
    fn range_operators() {
        assert_eq!(tok("1..10"),  vec![Token::Integer("1"), Token::DotDot,    Token::Integer("10")]);
        assert_eq!(tok("1...10"), vec![Token::Integer("1"), Token::DotDotDot, Token::Integer("10")]);
    }
}
