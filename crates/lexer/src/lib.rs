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
    #[token("spawn")]       Spawn,
    #[token("parallel")]    Parallel,
    #[token("do")]          Do,
    #[token("return")]      Return,
    #[token("import")]      Import,
    #[token("module")]      Module,
    #[token("export")]      Export,
    #[token("extern")]      Extern,
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
    #[token("ruleTest")]    RuleTest,
    #[token("validatorTest")] ValidatorTest,
    #[token("true")]        True,
    #[token("false")]       False,
    #[token("unit")]        Unit,
    #[token("in")]          In,
    #[token("is")]          Is,
    #[token("not")]         Not,
    #[token("and")]         And,
    #[token("or")]          Or,
    #[token("on")]          On,
    #[token("guard")]       Guard,
    #[token("require")]     Require,
    #[token("ensure")]      Ensure,
    #[token("defer")]       Defer,
    #[token("with")]        With,
    #[token("constraint")]  Constraint,
    #[token("temporal")]    Temporal,
    #[token("rule")]        Rule,
    #[token("after")]       After,
    #[token("overrides")]   Overrides,
    #[token("priority")]    Priority,
    #[token("trigger")]     Trigger,
    #[token("context")]     Context,
    #[token("errors")]      Errors,
    #[token("loaded")]      Loaded,

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
    /// True if a newline appears in the source between the previous token and
    /// this one. Used by the parser for newline-aware statement boundaries.
    pub newline_before: bool,
}

/// Lex a source string and collect all tokens (or errors).
///
/// Returns `Ok(Vec<Spanned>)` when every byte is recognised.
/// Returns `Err(Vec<(logos::Span, &str)>)` listing every unrecognised span.
pub fn lex(source: &str) -> Result<Vec<Spanned<'_>>, Vec<(logos::Span, &str)>> {
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    let mut prev_end = 0usize;
    for (result, span) in Token::lexer(source).spanned() {
        match result {
            Ok(token) => {
                // A newline in the skipped gap between the previous token and this
                // one marks a (potential) statement boundary.
                let newline_before = source[prev_end..span.start].contains('\n');
                prev_end = span.end;
                tokens.push(Spanned { token, span, newline_before });
            }
            Err(_) => {
                prev_end = span.end;
                errors.push((span.clone(), &source[span]));
            }
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

    // ------------------------------------------------------------------ //
    // Phase 1 — validator feature keywords
    // ------------------------------------------------------------------ //

    #[test]
    fn validator_feature_keywords() {
        assert_eq!(tok("constraint"),    vec![Token::Constraint]);
        assert_eq!(tok("temporal"),      vec![Token::Temporal]);
        assert_eq!(tok("rule"),          vec![Token::Rule]);
        assert_eq!(tok("after"),         vec![Token::After]);
        assert_eq!(tok("overrides"),     vec![Token::Overrides]);
        assert_eq!(tok("priority"),      vec![Token::Priority]);
        assert_eq!(tok("trigger"),       vec![Token::Trigger]);
        assert_eq!(tok("context"),       vec![Token::Context]);
        assert_eq!(tok("errors"),        vec![Token::Errors]);
        assert_eq!(tok("loaded"),        vec![Token::Loaded]);
        assert_eq!(tok("on"),            vec![Token::On]);
        assert_eq!(tok("ruleTest"),      vec![Token::RuleTest]);
        assert_eq!(tok("validatorTest"), vec![Token::ValidatorTest]);
    }

    #[test]
    fn validator_already_present() {
        // validator and require were already in the lexer — verify they still work
        assert_eq!(tok("validator"), vec![Token::Validator]);
        assert_eq!(tok("require"),   vec![Token::Require]);
    }

    #[test]
    fn keywords_do_not_swallow_adjacent_identifiers() {
        assert_eq!(tok("validator_name"), vec![Token::Ident("validator_name")]);
        assert_eq!(tok("rule_id"),        vec![Token::Ident("rule_id")]);
        assert_eq!(tok("context_data"),   vec![Token::Ident("context_data")]);
        assert_eq!(tok("after_tax"),      vec![Token::Ident("after_tax")]);
        assert_eq!(tok("errors_list"),    vec![Token::Ident("errors_list")]);
        assert_eq!(tok("loaded_by"),      vec![Token::Ident("loaded_by")]);
        assert_eq!(tok("trigger_fn"),     vec![Token::Ident("trigger_fn")]);
        assert_eq!(tok("priority_1"),     vec![Token::Ident("priority_1")]);
    }

    #[test]
    fn keywords_inside_strings_not_tokenised() {
        assert_eq!(tok(r#""validator""#),  vec![Token::StringLit("validator")]);
        assert_eq!(tok(r#""constraint""#), vec![Token::StringLit("constraint")]);
        assert_eq!(tok(r#""rule""#),       vec![Token::StringLit("rule")]);
        assert_eq!(tok(r#""context""#),    vec![Token::StringLit("context")]);
    }

    #[test]
    fn loaded_by_two_token_sequence() {
        // 'loaded by' is two tokens — the parser pairs them, not the lexer
        let tokens = tok("loaded by");
        assert_eq!(tokens, vec![Token::Loaded, Token::Ident("by")]);
    }

    #[test]
    fn not_in_two_token_sequence() {
        // 'not in' is two tokens — parser handles the operator pairing
        let tokens = tok("not in");
        assert_eq!(tokens, vec![Token::Not, Token::In]);
    }
}
