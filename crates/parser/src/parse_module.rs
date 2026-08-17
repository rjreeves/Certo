use certo_ast::module::{Module, Import, ImportKind, ImportedName, ImportCondition};
use certo_ast::span::{S, Span};
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};
use crate::parse_type::parse_module_path;
use crate::parse_decl::{parse_decl, parse_extern_block};

/// Top-level entry point: lex + parse a complete source file.
pub fn parse(source: &str) -> Result<Module, Vec<ParseError>> {
    let tokens = match certo_lexer::lex(source) {
        Ok(t) => t,
        Err(errs) => {
            return Err(errs.into_iter().map(|(span, text)| ParseError {
                kind: ParseErrorKind::Custom(format!("unrecognised token `{text}`")),
                span: span.into(),
            }).collect());
        }
    };

    let mut cur = Cursor::new(tokens);
    let mut errors = Vec::new();

    // `module MyApp.Orders.Processing`
    let path = match parse_module_path_decl(&mut cur) {
        Ok(p) => p,
        Err(e) => {
            errors.push(e);
            // recover with a dummy path
            certo_ast::types::ModulePath { segments: vec![], span: Span::DUMMY }
        }
    };

    // imports
    let mut imports = Vec::new();
    while cur.peek() == Some(&Token::Import)
        || (cur.peek() == Some(&Token::Pub) && cur.peek2() == Some(&Token::Import))
    {
        match parse_import(&mut cur) {
            Ok(i)  => imports.push(i),
            Err(e) => { errors.push(e); skip_to_next_decl(&mut cur); }
        }
    }

    // declarations
    let mut decls = Vec::new();
    while !cur.at_end() {
        // `extern "C" { ... }` expands to several body-less function decls.
        if cur.peek() == Some(&Token::Extern) {
            match parse_extern_block(&mut cur) {
                Ok(ds) => decls.extend(ds),
                Err(e) => { errors.push(e); skip_to_next_decl(&mut cur); }
            }
            continue;
        }
        match parse_decl(&mut cur) {
            Ok(ds) => decls.extend(ds),
            Err(e) => { errors.push(e); skip_to_next_decl(&mut cur); }
        }
    }

    if errors.is_empty() {
        Ok(Module { path, imports, decls, span: Span::DUMMY })
    } else {
        Err(errors)
    }
}

fn parse_module_path_decl(cur: &mut Cursor<'_>) -> Result<certo_ast::types::ModulePath, ParseError> {
    cur.expect(&Token::Module)?;
    parse_module_path(cur)
}

fn parse_import(cur: &mut Cursor<'_>) -> Result<Import, ParseError> {
    let start = cur.peek_span();
    let is_pub = cur.eat(|t| matches!(t, Token::Pub)).is_some();
    cur.expect(&Token::Import)?;

    // `import when [target = "wasm"] ...`
    let when = if let Some(Token::Ident(s)) = cur.peek() {
        if *s == "when" {
            cur.bump();
            cur.expect(&Token::LBracket)?;
            let (key, _) = cur.expect_ident()?;
            cur.expect(&Token::Eq)?;
            let (tok, tok_span) = cur.bump().ok_or(ParseError {
                kind: ParseErrorKind::UnexpectedEof, span: start,
            })?;
            let value = if let Token::StringLit(s) = tok { s.to_string() } else {
                return Err(ParseError {
                    kind: ParseErrorKind::Expected { expected: "string".into(), found: format!("{tok:?}") },
                    span: tok_span,
                });
            };
            cur.expect(&Token::RBracket)?;
            Some(ImportCondition { key, value, span: start })
        } else { None }
    } else { None };

    let path = parse_module_path(cur)?;

    // `as Alias`
    if cur.eat(|t| matches!(t, Token::As)).is_some() {
        let (alias, alias_span) = cur.expect_ident()?;
        let span = start.to(alias_span);
        return Ok(Import { is_pub, path, kind: ImportKind::Aliased(S::new(alias, alias_span)), when, span });
    }

    // `.{ Name, Name as Alias }`
    if cur.peek() == Some(&Token::Dot) && cur.peek2() == Some(&Token::LBrace) {
        cur.bump(); // eat `.`
        cur.bump(); // eat `{`
        let mut names = Vec::new();
        while cur.peek() != Some(&Token::RBrace) && !cur.at_end() {
            let (name, name_span) = cur.expect_ident()?;
            let alias = if cur.eat(|t| matches!(t, Token::As)).is_some() {
                let (a, as_) = cur.expect_ident()?;
                Some(S::new(a, as_))
            } else { None };
            let span = name_span;
            names.push(ImportedName { name: S::new(name, name_span), alias, span });
            if cur.eat(|t| matches!(t, Token::Comma)).is_none() { break; }
        }
        let end = cur.expect(&Token::RBrace)?;
        let span = start.to(end);
        return Ok(Import { is_pub, path, kind: ImportKind::Named(names), when, span });
    }

    let span = start.to(path.span);
    Ok(Import { is_pub, path, kind: ImportKind::Whole, when, span })
}

/// Skip tokens until we reach something that looks like the start of a new declaration.
fn skip_to_next_decl(cur: &mut Cursor<'_>) {
    loop {
        match cur.peek() {
            None
            | Some(Token::Fn)
            | Some(Token::Async)
            | Some(Token::Type)
            | Some(Token::Val)
            | Some(Token::Var)
            | Some(Token::Trait)
            | Some(Token::Impl)
            | Some(Token::StateMachine)
            | Some(Token::Migration)
            | Some(Token::View)
            | Some(Token::Test)
            | Some(Token::Property)
            | Some(Token::DbTest)
            | Some(Token::Validator)
            | Some(Token::Constraint)
            | Some(Token::Temporal)
            | Some(Token::RuleTest)
            | Some(Token::ValidatorTest)
            | Some(Token::Pub) => break,
            _ => { cur.bump(); }
        }
    }
}
