use certo_ast::module::{Module, Import, ImportKind, ImportedName, ImportCondition};
use certo_ast::span::{S, Span};
use certo_lexer::Token;
use crate::cursor::Cursor;
use crate::error::{ParseError, ParseErrorKind};
use crate::parse_type::parse_module_path;
use crate::parse_decl::{parse_decl, parse_extern_block};

/// Top-level entry point: lex + parse a complete source file.
///
/// BACKLOG item 357 - when two sum types in the file declare a variant with the same name, each
/// such variant is renamed `Variant_of_Type` (declaration and qualified uses `Type.Variant`), and an
/// unqualified use of the shared name is an error asking for the qualified form. Everything after
/// the parser then sees distinct variant names.
pub fn parse(source: &str) -> Result<Module, Vec<ParseError>> {
    let tokens = lex_or_errors(source)?;
    let module = parse_tokens(tokens.clone())?;
    let ambiguous = ambiguous_variants(&module);
    if ambiguous.is_empty() {
        return Ok(module);
    }
    let tokens = qualify_variants(tokens, &module, &ambiguous)?;
    parse_tokens(tokens)
}

/// Like `parse`, but never renames anything: for the formatter, which must print the source as
/// written.
pub fn parse_verbatim(source: &str) -> Result<Module, Vec<ParseError>> {
    parse_tokens(lex_or_errors(source)?)
}

fn lex_or_errors(source: &str) -> Result<Vec<certo_lexer::Spanned<'_>>, Vec<ParseError>> {
    match certo_lexer::lex(source) {
        Ok(t) => Ok(t),
        Err(errs) => Err(errs.into_iter().map(|(span, text)| ParseError {
            kind: ParseErrorKind::Custom(format!("unrecognised token `{text}`")),
            span: span.into(),
        }).collect()),
    }
}

/// Variant name -> the sum types (in declaration order) that declare it, for names declared more
/// than once.
fn ambiguous_variants(module: &Module) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut by_variant: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for d in &module.decls {
        if let certo_ast::decl::Decl::Type(t) = &d.node {
            if let certo_ast::decl::TypeBody::Sum(vs) = &t.body {
                for v in vs {
                    by_variant.entry(v.name.node.clone()).or_default().push(t.name.node.clone());
                }
            }
        }
    }
    by_variant.retain(|_, types| types.len() > 1);
    by_variant
}

fn qualify_variants<'src>(
    tokens: Vec<certo_lexer::Spanned<'src>>,
    module: &Module,
    ambiguous: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<Vec<certo_lexer::Spanned<'src>>, Vec<ParseError>> {
    // Declaration sites, by byte span.
    let mut decl_sites: std::collections::HashMap<(usize, usize), &'static str> = std::collections::HashMap::new();
    for d in &module.decls {
        if let certo_ast::decl::Decl::Type(t) = &d.node {
            if let certo_ast::decl::TypeBody::Sum(vs) = &t.body {
                for v in vs {
                    if ambiguous.contains_key(&v.name.node) {
                        let mangled: &'static str = Box::leak(format!("{}_of_{}", v.name.node, t.name.node).into_boxed_str());
                        decl_sites.insert((v.name.span.start as usize, v.name.span.end as usize), mangled);
                    }
                }
            }
        }
    }
    let declares = |ty: &str, variant: &str| ambiguous.get(variant).is_some_and(|ts| ts.iter().any(|t| t == ty));

    let mut out: Vec<certo_lexer::Spanned<'src>> = Vec::with_capacity(tokens.len());
    let mut errors = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Token::Ident(name) = &t.token {
            if let Some(m) = decl_sites.get(&(t.span.start, t.span.end)) {
                out.push(certo_lexer::Spanned { token: Token::Ident(m), span: t.span.clone(), newline_before: t.newline_before });
                i += 1;
                continue;
            }
            // `Type.Variant`
            if i + 2 < tokens.len() && tokens[i + 1].token == Token::Dot {
                if let Token::Ident(variant) = &tokens[i + 2].token {
                    if declares(name, variant) {
                        let m: &'static str = Box::leak(format!("{variant}_of_{name}").into_boxed_str());
                        out.push(certo_lexer::Spanned {
                            token: Token::Ident(m),
                            span: t.span.start..tokens[i + 2].span.end,
                            newline_before: t.newline_before,
                        });
                        i += 3;
                        continue;
                    }
                }
            }
            if let Some(types) = ambiguous.get(*name) {
                let after_dot = out.last().is_some_and(|p| p.token == Token::Dot);
                if !after_dot {
                    let qualified: Vec<String> = types.iter().map(|ty| format!("`{ty}.{name}`")).collect();
                    errors.push(ParseError {
                        kind: ParseErrorKind::Custom(format!(
                            "the variant `{name}` is declared by more than one type ({}) - write {}",
                            types.join(", "), qualified.join(" or "))),
                        span: t.span.clone().into(),
                    });
                }
            }
        }
        out.push(t.clone());
        i += 1;
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

fn parse_tokens(tokens: Vec<certo_lexer::Spanned<'_>>) -> Result<Module, Vec<ParseError>> {
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

    // `import when [target = "wasm"] ...` (spec §3.5, BACKLOG item 144) — `when`
    // is always lexed as the dedicated `Token::When` keyword (the same one
    // `parse_decl.rs`'s trigger-condition parsing already checks correctly),
    // never as a plain `Token::Ident("when")`, so this must match the real
    // token. Accepted-and-ignored, same treatment as item 149's `@valueObject`/
    // `@aggregate` annotations: the condition is parsed and stored on
    // `Import.when` (round-trips through `certo fmt`, which already formats
    // it defensively), but nothing downstream ever reads it to gate the
    // import — this compiler has no build-target concept for it to gate
    // against (`certo-wasm` is a real alternate backend, but a wholly
    // separate binary sharing no target/platform notion with the main
    // `certo` CLI; building real gating means inventing that concept from
    // scratch, a separate, larger, undesigned task — not attempted here).
    let when = if cur.peek() == Some(&Token::When) {
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
