use certo_lexer::{Token, Spanned};
use certo_ast::span::Span;
use crate::error::{ParseError, ParseErrorKind};

/// The token cursor shared by all parse functions.
pub struct Cursor<'src> {
    tokens: Vec<Spanned<'src>>,
    pos:    usize,
    /// When true, a trailing `{ … }` is NOT treated as a trailing-lambda call.
    /// Set while parsing a `match` scrutinee so `match c { Red => … }` reads the
    /// braces as the match body, not as `c { Red => … }` (a lambda call).
    suppress_trailing_lambda: bool,
}

impl<'src> Cursor<'src> {
    pub fn new(tokens: Vec<Spanned<'src>>) -> Self {
        Cursor { tokens, pos: 0, suppress_trailing_lambda: false }
    }

    /// Set the trailing-lambda suppression flag, returning the previous value
    /// (so callers can restore it).
    pub fn set_suppress_trailing_lambda(&mut self, v: bool) -> bool {
        std::mem::replace(&mut self.suppress_trailing_lambda, v)
    }

    // ---------------------------------------------------------------- //
    // Peeking
    // ---------------------------------------------------------------- //

    pub fn peek(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.pos).map(|s| &s.token)
    }

    /// True if the next token begins a new source line (a newline precedes it).
    /// Used to stop postfix operators (e.g. a call `(`) from spanning a line
    /// break, so `f(x)` on one line and `(y)` on the next are two statements.
    pub fn peek_newline_before(&self) -> bool {
        self.tokens.get(self.pos).map(|s| s.newline_before).unwrap_or(false)
    }

    pub fn peek2(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + 1).map(|s| &s.token)
    }

    pub fn peek3(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + 2).map(|s| &s.token)
    }

    pub fn peek4(&self) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + 3).map(|s| &s.token)
    }

    /// Peek `offset` tokens ahead of the current position (0 = current token).
    /// For unbounded lookahead scans where a fixed peek2/peek3/peek4 isn't enough —
    /// e.g. finding the token right after a balanced `(...)` group of arbitrary length.
    pub fn peek_at(&self, offset: usize) -> Option<&Token<'src>> {
        self.tokens.get(self.pos + offset).map(|s| &s.token)
    }

    /// True if the next tokens look like a trailing lambda: `{ ident => …` or `{ ident, ident => …`
    pub fn is_trailing_lambda(&self) -> bool {
        if self.suppress_trailing_lambda { return false; }
        if self.peek() != Some(&Token::LBrace) { return false; }
        match (self.peek2(), self.peek3()) {
            // `{ ident => …`
            (Some(Token::Ident(_)), Some(Token::FatArrow)) => true,
            // `{ ident, ident => …`  (two-param)
            (Some(Token::Ident(_)), Some(Token::Comma)) => {
                matches!(self.peek4(), Some(Token::Ident(_)))
                    && matches!(self.tokens.get(self.pos + 4).map(|s| &s.token), Some(Token::FatArrow))
            }
            _ => false,
        }
    }

    pub fn peek_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|s| s.span.clone().into())
            .unwrap_or(Span::DUMMY)
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    // ---------------------------------------------------------------- //
    // Consuming
    // ---------------------------------------------------------------- //

    /// Advance and return the next token + its span.
    pub fn bump(&mut self) -> Option<(Token<'src>, Span)> {
        if self.pos < self.tokens.len() {
            let s = &self.tokens[self.pos];
            let tok = s.token.clone();
            let span = s.span.clone().into();
            self.pos += 1;
            Some((tok, span))
        } else {
            None
        }
    }

    /// Consume the next token only if it matches the predicate; return its span.
    pub fn eat<F>(&mut self, f: F) -> Option<(Token<'src>, Span)>
    where
        F: Fn(&Token<'src>) -> bool,
    {
        if self.peek().map(f).unwrap_or(false) {
            self.bump()
        } else {
            None
        }
    }

    /// Consume the next token if it is exactly `expected`, or return an error.
    pub fn expect(&mut self, expected: &Token<'static>) -> Result<Span, ParseError> {
        match self.peek() {
            Some(t) if token_matches(t, expected) => {
                let (_, span) = self.bump().unwrap();
                Ok(span)
            }
            other => {
                let span = self.peek_span();
                let found = other.map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
                Err(ParseError {
                    kind: ParseErrorKind::Expected {
                        expected: format!("{expected:?}"),
                        found,
                    },
                    span,
                })
            }
        }
    }

    /// Consume an identifier token; return its text and span.
    pub fn expect_ident(&mut self) -> Result<(String, Span), ParseError> {
        match self.peek() {
            Some(Token::Ident(_)) => {
                let (tok, span) = self.bump().unwrap();
                if let Token::Ident(name) = tok {
                    Ok((name.to_string(), span))
                } else {
                    unreachable!()
                }
            }
            other => {
                let span = self.peek_span();
                let found = other.map(|t| format!("{t:?}")).unwrap_or_else(|| "end of file".into());
                Err(ParseError {
                    kind: ParseErrorKind::Expected {
                        expected: "identifier".into(),
                        found,
                    },
                    span,
                })
            }
        }
    }

}

/// Structural token equality ignoring payload.
fn token_matches(a: &Token<'_>, b: &Token<'static>) -> bool {
    use Token::*;
    matches!(
        (a, b),
        (LParen,    LParen)    |
        (RParen,    RParen)    |
        (LBrace,    LBrace)    |
        (RBrace,    RBrace)    |
        (LBracket,  LBracket)  |
        (RBracket,  RBracket)  |
        (Comma,     Comma)     |
        (Colon,     Colon)     |
        (Semi,      Semi)      |
        (Dot,       Dot)       |
        (DotDot,    DotDot)    |
        (DotDotDot, DotDotDot) |
        (Pipe,      Pipe)      |
        (Bar,       Bar)       |
        (FatArrow,  FatArrow)  |
        (Arrow,     Arrow)     |
        (Eq,        Eq)        |
        (EqEq,      EqEq)      |
        (NotEq,     NotEq)     |
        (Lt,        Lt)        |
        (LtEq,      LtEq)      |
        (Gt,        Gt)        |
        (GtEq,      GtEq)      |
        (Plus,      Plus)      |
        (Minus,     Minus)     |
        (Star,      Star)      |
        (Slash,     Slash)     |
        (Percent,   Percent)   |
        (StarStar,  StarStar)  |
        (Question,  Question)  |
        (SafeDot,   SafeDot)   |
        (Bang,      Bang)      |
        (At,        At)        |
        (Hash,      Hash)      |
        (Fn,        Fn)        |
        (Type,      Type)      |
        (Let,       Let)       |
        (Val,       Val)       |
        (Var,       Var)       |
        (Match,     Match)     |
        (If,        If)        |
        (Then,      Then)      |
        (Else,      Else)      |
        (When,      When)      |
        (Async,     Async)     |
        (Await,     Await)     |
        (Spawn,     Spawn)     |
        (Parallel,  Parallel)  |
        (Do,        Do)        |
        (Return,    Return)    |
        (Import,    Import)    |
        (Module,    Module)    |
        (Export,    Export)    |
        (Extern,    Extern)    |
        (Pub,       Pub)       |
        (Priv,      Priv)      |
        (Trait,     Trait)     |
        (Impl,      Impl)      |
        (For,       For)       |
        (While,     While)     |
        (Where,     Where)     |
        (As,        As)        |
        (StateMachine, StateMachine) |
        (Validator, Validator) |
        (Migration, Migration) |
        (View,      View)      |
        (Form,      Form)      |
        (Test,      Test)      |
        (Property,  Property)  |
        (DbTest,    DbTest)    |
        (True,      True)      |
        (False,     False)     |
        (Unit,      Unit)      |
        (In,        In)        |
        (Is,        Is)        |
        (Not,       Not)       |
        (And,       And)       |
        (Or,        Or)        |
        (Guard,     Guard)     |
        (Require,   Require)   |
        (Ensure,    Ensure)    |
        (Defer,     Defer)     |
        (Use,       Use)       |
        (With,      With)      |
        (On,        On)        |
        (Constraint, Constraint) |
        (Temporal,  Temporal)  |
        (Rule,      Rule)      |
        (After,     After)     |
        (Overrides, Overrides) |
        (Priority,  Priority)  |
        (Trigger,   Trigger)   |
        (Context,   Context)   |
        (Errors,    Errors)    |
        (Loaded,    Loaded)    |
        (RuleTest,  RuleTest)  |
        (ValidatorTest, ValidatorTest)
    )
}
