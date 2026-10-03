//! Recursive-descent SDL parser with precedence climbing for expressions.
//! Errors are collected; after a bad declaration the parser resynchronises
//! at the next top-level keyword so one typo doesn't hide later errors.

// `Err(())` deliberately carries nothing: the diagnostic is recorded on the
// parser before the error is returned.
#![allow(clippy::result_unit_err)]

use crate::ast::*;
use crate::lexer::{lex, TokKind, Token};
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;

/// Parse SDL source into an AST plus all diagnostics (lexical and syntactic).
/// The AST contains every declaration that parsed successfully.
pub fn parse(src: &str) -> (SdlFile, Vec<Diagnostic>) {
    let (toks, mut diags) = lex(src);
    let mut p = Parser { toks, pos: 0, diags: Vec::new(), expr_depth: 0, expr_nodes: 0 };
    let file = p.file();
    diags.extend(p.diags);
    (file, diags)
}

/// `Err(())` means a diagnostic was already recorded.
pub type PResult<T> = Result<T, ()>;

const DECL_KEYWORDS: &[&str] = &["table", "enum", "type", "index", "constraint", "sequence"];

/// Largest expression (operands + operators) accepted. Bounds recursion depth
/// in the parser, type checker and drop glue, so the compiler is safe to call
/// from a host thread with a small stack.
const MAX_EXPR_NODES: usize = 500;

/// Deepest expression tree accepted. Type checking recurses along the tree
/// (about 4-5 KB of stack per level in an unoptimised build), so this bounds
/// its stack use on any host. Long flat `in` lists and argument lists add
/// elements, not depth.
const MAX_TREE_DEPTH: usize = 128;

/// Deepest nesting of parentheses, call arguments and `in` lists. Parser
/// recursion follows nesting, so this is what keeps it inside a small stack;
/// real expressions nest a handful of levels.
const MAX_EXPR_DEPTH: usize = 64;

/// Token cursor plus the shared building blocks (identifiers, expressions,
/// errors). SDL's own grammar lives here; sibling languages such as MDL reuse
/// it through the public methods.
pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    expr_depth: usize,
    expr_nodes: usize,
}

impl Parser {
    /// Lex `src`; lexical diagnostics are returned with the parser and count
    /// towards `into_diagnostics`.
    pub fn new(src: &str) -> Parser {
        let (toks, diags) = lex(src);
        Parser { toks, pos: 0, diags, expr_depth: 0, expr_nodes: 0 }
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> { self.diags }

    pub fn at_eof(&self) -> bool { *self.peek() == TokKind::Eof }

    /// Token index; lets callers detect "no progress" while recovering.
    pub fn pos(&self) -> usize { self.pos }

    pub fn report(&mut self, d: Diagnostic) { self.diags.push(d); }

    pub fn string_lit(&mut self, what: &str) -> PResult<(String, Span)> {
        if let TokKind::Str(s) = self.peek() {
            let s = s.clone();
            let span = self.bump().span;
            Ok((s, span))
        } else {
            self.err(what)
        }
    }

    /// Skip to the next identifier in `words` that is outside any braces
    /// (used to resynchronise after an error).
    pub fn recover_to(&mut self, words: &[&str]) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                TokKind::Eof => return,
                TokKind::LBrace => depth += 1,
                TokKind::RBrace => depth = (depth - 1).max(0),
                TokKind::Ident(s) if depth == 0 && words.contains(&s.as_str()) => return,
                _ => {}
            }
            self.bump();
        }
    }

    pub fn peek(&self) -> &TokKind { &self.toks[self.pos].kind }

    pub fn peek_at(&self, n: usize) -> &TokKind {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].kind
    }

    pub fn span(&self) -> Span { self.toks[self.pos].span }

    pub fn prev_span(&self) -> Span { self.toks[self.pos.saturating_sub(1)].span }

    pub fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 { self.pos += 1; }
        t
    }

    pub fn at_word(&self, w: &str) -> bool {
        matches!(self.peek(), TokKind::Ident(s) if s == w)
    }

    pub fn eat_word(&mut self, w: &str) -> bool {
        let hit = self.at_word(w);
        if hit { self.bump(); }
        hit
    }

    pub fn eat(&mut self, k: &TokKind) -> bool {
        let hit = self.peek() == k;
        if hit { self.bump(); }
        hit
    }

    fn describe(k: &TokKind) -> String {
        match k {
            TokKind::Ident(s) => format!("`{s}`"),
            TokKind::Str(_) => "string".into(),
            TokKind::Num(n) => format!("`{n}`"),
            TokKind::Dec(d) => format!("`{d}`"),
            TokKind::Eof => "end of file".into(),
            other => format!("`{}`", symbol(other)),
        }
    }

    pub fn err<T>(&mut self, expected: &str) -> PResult<T> {
        let found = Self::describe(self.peek());
        self.diags.push(
            Diagnostic::error("SDL100", format!("expected {expected}, found {found}"))
                .with_span(self.span())
                .with_label(format!("expected {expected}")),
        );
        Err(())
    }

    pub fn expect(&mut self, k: TokKind) -> PResult<Span> {
        if *self.peek() == k { Ok(self.bump().span) }
        else { self.err(&format!("`{}`", symbol(&k))) }
    }

    pub fn expect_word(&mut self, w: &str) -> PResult<Span> {
        if self.at_word(w) { Ok(self.bump().span) }
        else { self.err(&format!("`{w}`")) }
    }

    pub fn ident(&mut self, what: &str) -> PResult<Ident> {
        if let TokKind::Ident(s) = self.peek() {
            let name = s.clone();
            let span = self.bump().span;
            Ok(Ident { name, span })
        } else {
            self.err(what)
        }
    }

    // ---- declarations ------------------------------------------------ //

    fn file(&mut self) -> SdlFile {
        let mut decls = Vec::new();
        while *self.peek() != TokKind::Eof {
            match self.decl() {
                Ok(d) => decls.push(d),
                Err(()) => self.recover(),
            }
        }
        SdlFile { decls }
    }

    fn at_decl_start(&self) -> bool {
        matches!(self.peek(), TokKind::Ident(s) if DECL_KEYWORDS.contains(&s.as_str()))
            && !matches!(self.peek_at(1), TokKind::Colon)
    }

    /// Skip to the next top-level declaration keyword (outside any braces).
    fn recover(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.peek() {
                TokKind::Eof => return,
                TokKind::LBrace => depth += 1,
                TokKind::RBrace => depth = (depth - 1).max(0),
                TokKind::Ident(s)
                    if depth == 0
                        && DECL_KEYWORDS.contains(&s.as_str())
                        && !matches!(self.peek_at(1), TokKind::Colon) =>
                {
                    return;
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn decl(&mut self) -> PResult<Decl> {
        let kw = match self.peek() {
            TokKind::Ident(s) if DECL_KEYWORDS.contains(&s.as_str()) => s.clone(),
            _ => return self.err("a declaration (`table`, `enum`, `type`, `index`, `constraint` or `sequence`)"),
        };
        let start = self.bump().span;
        match kw.as_str() {
            "table" => self.table(start).map(Decl::Table),
            "enum" => self.enum_decl(start).map(Decl::Enum),
            "type" => self.type_decl(start).map(Decl::Type),
            "index" => self.index(start).map(Decl::Index),
            "sequence" => self.sequence(start).map(Decl::Sequence),
            _ => self.constraint(start).map(Decl::Constraint),
        }
    }

    fn table(&mut self, start: Span) -> PResult<TableDecl> {
        let name = self.ident("a table name")?;
        self.expect(TokKind::LBrace)?;
        let mut members = Vec::new();
        while *self.peek() != TokKind::RBrace {
            // A declaration keyword here means the closing `}` is missing;
            // stop without consuming it so recovery resumes at this decl.
            if self.at_decl_start() { return self.err("`}`"); }
            members.push(self.member()?);
        }
        let end = self.expect(TokKind::RBrace)?;
        Ok(TableDecl { name, members, span: start.to(end) })
    }

    fn member(&mut self) -> PResult<Member> {
        let name = self.ident("a column or relationship name")?;
        let start = name.span;
        self.expect(TokKind::Colon)?;
        let ty = self.type_ref()?;
        if self.eat(&TokKind::Arrow) {
            if !ty.args.is_empty() {
                self.diags.push(Diagnostic::error("SDL100", "a relationship target takes no parameters").with_span(ty.span));
                return Err(());
            }
            let cardinality = if self.eat_word("one") { Cardinality::One }
                else if self.eat_word("optional") { Cardinality::Optional }
                else if self.eat_word("many") { Cardinality::Many }
                else { return self.err("`one`, `optional` or `many`"); };
            return Ok(Member::Rel(RelDecl {
                name, target: ty.name, cardinality, span: start.to(self.prev_span()),
            }));
        }
        let mut mods = Vec::new();
        while let Some(m) = self.column_mod()? {
            mods.push(m);
        }
        Ok(Member::Column(ColumnDecl { name, ty, mods, span: start.to(self.prev_span()) }))
    }

    /// A modifier word only counts if it isn't the start of the next member
    /// (`word :`), so columns may be named `unique`, `null`, etc.
    fn column_mod(&mut self) -> PResult<Option<ColumnMod>> {
        if matches!(self.peek_at(1), TokKind::Colon) { return Ok(None); }
        let TokKind::Ident(w) = self.peek() else { return Ok(None) };
        let start = self.span();
        Ok(Some(match w.as_str() {
            "primary" => { self.bump(); self.expect_word("key")?; ColumnMod::PrimaryKey(start.to(self.prev_span())) }
            "unique" => { self.bump(); ColumnMod::Unique(start) }
            "not" => { self.bump(); self.expect_word("null")?; ColumnMod::NotNull(start.to(self.prev_span())) }
            "null" => { self.bump(); ColumnMod::Null(start) }
            "default" => { self.bump(); ColumnMod::Default(self.expr()?) }
            "generated" => {
                self.bump();
                let kind = if self.eat_word("always") {
                    Generation::Always
                } else {
                    self.expect_word("by")?;
                    self.expect_word("default")?;
                    Generation::ByDefault
                };
                ColumnMod::Generated { kind, span: start.to(self.prev_span()) }
            }
            "references" => {
                self.bump();
                let table = self.ident("a table name")?;
                let column = if self.eat(&TokKind::LParen) {
                    let c = self.ident("a column name")?;
                    self.expect(TokKind::RParen)?;
                    Some(c)
                } else {
                    None
                };
                let (mut on_delete, mut on_update) = (None, None);
                while self.at_word("on")
                    && matches!(self.peek_at(1), TokKind::Ident(w) if w == "delete" || w == "update")
                {
                    self.bump();
                    let is_delete = self.at_word("delete");
                    let kw = self.bump().span;
                    let action = self.ref_action()?;
                    let slot = if is_delete { &mut on_delete } else { &mut on_update };
                    if slot.is_some() {
                        self.diags.push(
                            Diagnostic::error("SDL207", "duplicate `on delete` / `on update` clause").with_span(kw),
                        );
                        return Err(());
                    }
                    *slot = Some(action);
                }
                ColumnMod::References { table, column, on_delete, on_update, span: start.to(self.prev_span()) }
            }
            _ => return Ok(None),
        }))
    }

    /// `name` or `name(n)` / `name(p, s)`.
    pub fn type_ref(&mut self) -> PResult<TypeRef> {
        let name = self.ident("a type name")?;
        let mut args = Vec::new();
        let mut end = name.span;
        if self.eat(&TokKind::LParen) {
            loop {
                match self.peek().clone() {
                    TokKind::Num(n) => {
                        let Ok(v) = u32::try_from(n) else {
                            self.diags.push(Diagnostic::error("SDL002", "type parameter is too large").with_span(self.span()));
                            return Err(());
                        };
                        self.bump();
                        args.push(v);
                    }
                    _ => return self.err("a number"),
                }
                if !self.eat(&TokKind::Comma) { break; }
            }
            end = self.expect(TokKind::RParen)?;
        }
        Ok(TypeRef { span: name.span.to(end), name, args })
    }

    fn ref_action(&mut self) -> PResult<ReferentialAction> {
        if self.eat_word("cascade") {
            Ok(ReferentialAction::Cascade)
        } else if self.eat_word("restrict") {
            Ok(ReferentialAction::Restrict)
        } else if self.eat_word("set") {
            self.expect_word("null")?;
            Ok(ReferentialAction::SetNull)
        } else if self.eat_word("no") {
            self.expect_word("action")?;
            Ok(ReferentialAction::NoAction)
        } else {
            self.err("`cascade`, `restrict`, `set null` or `no action`")
        }
    }

    fn enum_decl(&mut self, start: Span) -> PResult<EnumDecl> {
        let name = self.ident("an enum name")?;
        self.expect(TokKind::LBrace)?;
        let mut variants = vec![self.ident("an enum variant")?];
        while self.eat(&TokKind::Comma) {
            variants.push(self.ident("an enum variant")?);
        }
        let end = self.expect(TokKind::RBrace)?;
        Ok(EnumDecl { name, variants, span: start.to(end) })
    }

    fn type_decl(&mut self, start: Span) -> PResult<TypeDecl> {
        let name = self.ident("a type name")?;
        self.expect(TokKind::LBrace)?;
        let mut fields = Vec::new();
        loop {
            let fname = self.ident("a field name")?;
            self.expect(TokKind::Colon)?;
            let ty = self.type_ref()?;
            let span = fname.span.to(ty.span);
            fields.push(FieldDecl { name: fname, ty, span });
            if *self.peek() == TokKind::RBrace { break; }
        }
        let end = self.expect(TokKind::RBrace)?;
        Ok(TypeDecl { name, fields, span: start.to(end) })
    }

    /// An optionally negative integer.
    fn signed_int(&mut self) -> PResult<i64> {
        let neg = self.eat(&TokKind::Minus);
        match self.peek().clone() {
            TokKind::Num(n) => {
                let v = if neg { -(n as i128) } else { n as i128 };
                match i64::try_from(v) {
                    Ok(v) => { self.bump(); Ok(v) }
                    Err(_) => {
                        self.diags.push(Diagnostic::error("SDL002", "number is too large").with_span(self.span()));
                        Err(())
                    }
                }
            }
            _ => self.err("a whole number"),
        }
    }

    fn sequence(&mut self, start: Span) -> PResult<SequenceDecl> {
        let name = self.ident("a sequence name")?;
        let mut options = Vec::new();
        let mut end = name.span;
        while let TokKind::Ident(w) = self.peek().clone() {
            let kind = match w.as_str() {
                "start" => SeqOptKind::Start,
                "increment" => SeqOptKind::Increment,
                "min" => SeqOptKind::Min,
                "max" => SeqOptKind::Max,
                "cache" => SeqOptKind::Cache,
                "cycle" => SeqOptKind::Cycle,
                _ => break,
            };
            let span = self.span();
            self.bump();
            let value = if kind == SeqOptKind::Cycle { None } else { Some(self.signed_int()?) };
            end = self.prev_span();
            options.push(SeqOption { kind, value, span: span.to(end) });
        }
        Ok(SequenceDecl { name, options, span: start.to(end) })
    }

    fn index(&mut self, start: Span) -> PResult<IndexDecl> {
        let name = self.ident("an index name")?;
        self.expect_word("on")?;
        let table = self.ident("a table name")?;
        self.expect(TokKind::LParen)?;
        let mut columns = vec![self.ident("a column name")?];
        while self.eat(&TokKind::Comma) {
            columns.push(self.ident("a column name")?);
        }
        let end = self.expect(TokKind::RParen)?;
        Ok(IndexDecl { name, table, columns, span: start.to(end) })
    }

    fn constraint(&mut self, start: Span) -> PResult<ConstraintDecl> {
        let name = self.ident("a constraint name")?;
        self.expect_word("on")?;
        let table = self.ident("a table name")?;
        self.expect_word("using")?;
        let expr = self.expr()?;
        let span = start.to(expr.span());
        Ok(ConstraintDecl { name, table, expr, span })
    }

    // ---- expressions ------------------------------------------------- //

    pub fn expr(&mut self) -> PResult<Expr> {
        if self.expr_depth == 0 { self.expr_nodes = 0; }
        if self.expr_depth >= MAX_EXPR_DEPTH {
            self.diags.push(
                Diagnostic::error("SDL101", format!("expression is nested too deeply (more than {MAX_EXPR_DEPTH} levels)"))
                    .with_span(self.span()),
            );
            return Err(());
        }
        self.expr_depth += 1;
        let r = self.expr_bp(1);
        self.expr_depth -= 1;
        r
    }

    fn count_node(&mut self) -> PResult<()> {
        self.expr_nodes += 1;
        if self.expr_nodes > MAX_EXPR_NODES {
            self.diags.push(
                Diagnostic::error("SDL101", format!("expression is too large (more than {MAX_EXPR_NODES} elements)"))
                    .with_span(self.span()),
            );
            return Err(());
        }
        Ok(())
    }

    fn binop(&self) -> Option<BinaryOp> {
        Some(match self.peek() {
            TokKind::EqEq => BinaryOp::Eq,
            TokKind::NotEq => BinaryOp::Ne,
            TokKind::Lt => BinaryOp::Lt,
            TokKind::Le => BinaryOp::Le,
            TokKind::Gt => BinaryOp::Gt,
            TokKind::Ge => BinaryOp::Ge,
            TokKind::Plus => BinaryOp::Add,
            TokKind::Minus => BinaryOp::Sub,
            TokKind::Star => BinaryOp::Mul,
            TokKind::Slash => BinaryOp::Div,
            // `and`/`or` are operators unless they start the next member.
            TokKind::Ident(s) if !matches!(self.peek_at(1), TokKind::Colon) => match s.as_str() {
                "and" => BinaryOp::And,
                "or" => BinaryOp::Or,
                _ => return None,
            },
            _ => return None,
        })
    }

    /// True at `word` when it is an operator, not the start of the next member (`word :`).
    fn op_word(&self, w: &str) -> bool {
        self.at_word(w) && !matches!(self.peek_at(1), TokKind::Colon)
    }

    fn expr_bp(&mut self, min: u8) -> PResult<Expr> {
        // prefix `not` binds looser than comparison, tighter than `and`
        let mut lhs = if min <= 3
            && self.op_word("not")
            && !matches!(self.peek_at(1), TokKind::Ident(w) if w == "null")
        {
            let start = self.span();
            self.count_node()?;
            self.bump();
            // prefix `not` recurses before any node exists to be measured, so guard it directly
            if self.expr_depth >= MAX_EXPR_DEPTH {
                self.diags.push(
                    Diagnostic::error("SDL101", format!("expression is nested too deeply (more than {MAX_EXPR_DEPTH} levels)"))
                        .with_span(self.span()),
                );
                return Err(());
            }
            self.expr_depth += 1;
            let operand = self.expr_bp(3);
            self.expr_depth -= 1;
            let operand = operand?;
            let span = start.to(operand.span());
            let e = Expr::Not(Box::new(operand), span);
            self.check_depth(&e)?;
            e
        } else {
            self.primary()?
        };
        loop {
            // postfix predicates live at comparison level
            if min <= 4 && self.op_word("is") {
                self.count_node()?;
                self.bump();
                let negated = self.eat_word("not");
                let end = self.expect_word("null")?;
                let span = lhs.span().to(end);
                lhs = Expr::IsNull { expr: Box::new(lhs), negated, span };
                self.check_depth(&lhs)?;
                continue;
            }
            if min <= 4
                && (self.op_word("in")
                    || (self.op_word("not") && matches!(self.peek_at(1), TokKind::Ident(w) if w == "in")))
            {
                self.count_node()?;
                let negated = self.eat_word("not");
                self.expect_word("in")?;
                self.expect(TokKind::LParen)?;
                let mut list = vec![self.expr()?];
                while self.eat(&TokKind::Comma) {
                    list.push(self.expr()?);
                }
                let end = self.expect(TokKind::RParen)?;
                let span = lhs.span().to(end);
                lhs = Expr::In { expr: Box::new(lhs), list, negated, span };
                self.check_depth(&lhs)?;
                continue;
            }
            let Some(op) = self.binop() else { break };
            if op.precedence() < min { break; }
            self.count_node()?;
            self.bump();
            let rhs = self.expr_bp(op.precedence() + 1)?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
            self.check_depth(&lhs)?;
        }
        Ok(lhs)
    }

    /// Depth of an already-built (and therefore already bounded) expression.
    fn tree_depth(e: &Expr) -> usize {
        match e {
            Expr::Literal(..) | Expr::Ident(_) => 1,
            Expr::Paren(i, _) | Expr::Not(i, _) => 1 + Self::tree_depth(i),
            Expr::IsNull { expr, .. } => 1 + Self::tree_depth(expr),
            Expr::Call { args, .. } => 1 + args.iter().map(Self::tree_depth).max().unwrap_or(0),
            Expr::Binary { lhs, rhs, .. } => 1 + Self::tree_depth(lhs).max(Self::tree_depth(rhs)),
            Expr::In { expr, list, .. } => 1 + Self::tree_depth(expr).max(list.iter().map(Self::tree_depth).max().unwrap_or(0)),
        }
    }

    fn check_depth(&mut self, e: &Expr) -> PResult<()> {
        if Self::tree_depth(e) > MAX_TREE_DEPTH {
            self.diags.push(
                Diagnostic::error("SDL101", format!("expression is too deep (more than {MAX_TREE_DEPTH} levels of operators)"))
                    .with_span(e.span()),
            );
            return Err(());
        }
        Ok(())
    }

    fn too_large<T>(&mut self, span: Span) -> PResult<T> {
        self.diags.push(Diagnostic::error("SDL002", "number literal is too large").with_span(span));
        Err(())
    }

    fn primary(&mut self) -> PResult<Expr> {
        self.count_node()?;
        let span = self.span();
        match self.peek().clone() {
            TokKind::Num(n) => {
                self.bump();
                match i64::try_from(n) {
                    Ok(v) => Ok(Expr::Literal(Literal::Number(v), span)),
                    Err(_) => self.too_large(span),
                }
            }
            TokKind::Dec(d) => { self.bump(); Ok(Expr::Literal(Literal::Decimal(d), span)) }
            // a leading minus directly in front of a number is a negative literal
            TokKind::Minus if matches!(self.peek_at(1), TokKind::Num(_) | TokKind::Dec(_)) => {
                self.bump();
                let lit_span = span.to(self.span());
                match self.bump().kind {
                    TokKind::Num(n) => match i64::try_from(-(n as i128)) {
                        Ok(v) => Ok(Expr::Literal(Literal::Number(v), lit_span)),
                        Err(_) => self.too_large(lit_span),
                    },
                    TokKind::Dec(d) => Ok(Expr::Literal(Literal::Decimal(format!("-{d}")), lit_span)),
                    _ => unreachable!("peeked a number"),
                }
            }
            TokKind::Str(s) => { self.bump(); Ok(Expr::Literal(Literal::String(s), span)) }
            TokKind::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                Ok(Expr::Literal(Literal::Bool(s == "true"), span))
            }
            TokKind::Ident(_) => {
                let id = self.ident("an expression")?;
                if !self.eat(&TokKind::LParen) { return Ok(Expr::Ident(id)); }
                let mut args = Vec::new();
                if *self.peek() != TokKind::RParen {
                    args.push(self.expr()?);
                    while self.eat(&TokKind::Comma) { args.push(self.expr()?); }
                }
                let end = self.expect(TokKind::RParen)?;
                let span = id.span.to(end);
                Ok(Expr::Call { func: id, args, span })
            }
            TokKind::LParen => {
                self.bump();
                let e = self.expr()?;
                let end = self.expect(TokKind::RParen)?;
                Ok(Expr::Paren(Box::new(e), span.to(end)))
            }
            _ => self.err("an expression"),
        }
    }
}

fn symbol(k: &TokKind) -> &'static str {
    match k {
        TokKind::LBrace => "{", TokKind::RBrace => "}",
        TokKind::LParen => "(", TokKind::RParen => ")",
        TokKind::Colon => ":", TokKind::Comma => ",", TokKind::Arrow => "->",
        TokKind::Dot => ".", TokKind::Eq => "=",
        TokKind::EqEq => "==", TokKind::NotEq => "!=",
        TokKind::Lt => "<", TokKind::Le => "<=", TokKind::Gt => ">", TokKind::Ge => ">=",
        TokKind::Plus => "+", TokKind::Minus => "-", TokKind::Star => "*", TokKind::Slash => "/",
        TokKind::Concat => "||",
        _ => "?",
    }
}
