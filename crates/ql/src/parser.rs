//! QL parser, built on the SDL lexer and cursor.
//!
//! Clause order is fixed: `from`, `join`s, `where`, `group by`, `having`, `window`,
//! `select`, `order by`, `limit`, `offset`. Keywords are contextual.
//!
//! Expressions are bounded (elements, tree depth, nesting) so type checking
//! and lowering stay within a small stack on any host.

#![allow(clippy::result_unit_err)]

use crate::ast::*;
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use certo_sdl::{BinaryOp, PResult, Parser, TokKind};

const MAX_NODES: usize = 500;
const MAX_TREE_DEPTH: usize = 128;
const MAX_NESTING: usize = 64;

/// Words that end a table reference, so they are never taken as its alias.
const CLAUSE_WORDS: &[&str] = &[
    "inner", "left", "join", "on", "where", "group", "having", "select", "order", "limit", "offset", "set", "all",
    "returning", "values", "from", "window",
];

/// How many subqueries may sit inside one another.
const MAX_SUBQUERY_DEPTH: usize = 6;

pub fn parse(src: &str) -> (QlFile, Vec<Diagnostic>) {
    let mut q = Q { p: Parser::new(src), nodes: 0, nesting: 0, sub_depth: 0, windows: Vec::new() };
    let (mut queries, mut mutations, mut fragments, mut views) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    while !q.p.at_eof() {
        let start = q.p.pos();
        let result = if q.p.at_word("insert") {
            q.mutation(MutationKind::Insert).map(|m| mutations.push(m))
        } else if q.p.at_word("update") {
            q.mutation(MutationKind::Update).map(|m| mutations.push(m))
        } else if q.p.at_word("delete") {
            q.mutation(MutationKind::Delete).map(|m| mutations.push(m))
        } else if q.p.at_word("fragment") {
            q.fragment().map(|f| fragments.push(f))
        } else if q.p.at_word("view") {
            q.view().map(|v| views.push(v))
        } else {
            q.query().map(|x| queries.push(x))
        };
        if result.is_err() {
            if q.p.pos() == start {
                q.p.bump();
            }
            q.p.recover_to(&["query", "insert", "update", "delete", "fragment", "view"]);
        }
    }
    (QlFile { queries, mutations, fragments, views }, q.p.into_diagnostics())
}

struct Q {
    p: Parser,
    nodes: usize,
    nesting: usize,
    sub_depth: usize,
    /// The `window name as (...)` definitions of the query being parsed: `over name` is replaced by the
    /// definition as it is read, so nothing after the parser knows about names.
    windows: Vec<(String, WindowSpec)>,
}

impl Q {
    fn word_is(&self, w: &str) -> bool { self.p.at_word(w) }

    /// A keyword in operator position. (SDL guards against `word :` meaning a member
    /// declaration; in QL a `:` after a keyword starts a parameter: `like :pattern`.)
    fn op_word(&self, w: &str) -> bool { self.p.at_word(w) }

    fn fail<T>(&mut self, code: &str, msg: impl Into<String>, span: Span) -> PResult<T> {
        self.p.report(Diagnostic::error(code, msg).with_span(span));
        Err(())
    }

    // ---- query ------------------------------------------------------------ //

    fn query(&mut self) -> PResult<Query> {
        let start = self.p.span();
        self.p.expect_word("query")?;
        let name = self.p.ident("a query name")?;
        let params = self.param_list()?;
        self.p.expect(TokKind::LBrace)?;
        let mut q = self.query_body(name, params)?;
        let end = self.p.expect(TokKind::RBrace)?;
        q.span = start.to(end);
        Ok(q)
    }

    /// `fragment name(p: type, ...) { from ... select ... }`: a named query used as a table; its parameters are given
    /// as arguments where it is used.
    fn fragment(&mut self) -> PResult<Fragment> {
        let start = self.p.span();
        self.p.expect_word("fragment")?;
        let name = self.p.ident("a fragment name")?;
        let params = self.param_list()?;
        self.p.expect(TokKind::LBrace)?;
        let mut query = self.query_body(name.clone(), params)?;
        let end = self.p.expect(TokKind::RBrace)?;
        query.span = start.to(end);
        Ok(Fragment { name, query, span: start.to(end) })
    }

    /// `view name { from ... select ... }`: a query kept in the database as a view. It takes no parameters (`view name() { }` is
    /// accepted too): a view is a table, and a table has none.
    fn view(&mut self) -> PResult<ViewDecl> {
        let start = self.p.span();
        self.p.expect_word("view")?;
        let name = self.p.ident("a view name")?;
        if *self.p.peek() == TokKind::LParen {
            let params = self.param_list()?;
            if let Some(p) = params.first() {
                return self.fail("QL262", "a view takes no parameters: it is kept in the database as a table (use a `query` for parameters)", p.name.span);
            }
        }
        self.p.expect(TokKind::LBrace)?;
        let mut query = self.query_body(name.clone(), Vec::new())?;
        let end = self.p.expect(TokKind::RBrace)?;
        query.span = start.to(end);
        Ok(ViewDecl { name, query, span: start.to(end) })
    }

    /// `from ... select ... [union ...] [order by] [limit] [offset]`: a query's clauses, which
    /// are also a subquery's and an `insert ... select`'s.
    fn query_body(&mut self, name: certo_sdl::Ident, params: Vec<ParamDecl>) -> PResult<Query> {
        let start = self.p.span();
        let ctes = if self.word_is("with") { self.ctes()? } else { Vec::new() };
        let mut q = self.branch(name, params)?;
        q.ctes = ctes;
        while self.word_is("union") || self.word_is("intersect") || self.word_is("except") {
            let bstart = self.p.span();
            let op = match self.p.bump().kind {
                TokKind::Ident(w) if w == "union" => SetOp::Union,
                TokKind::Ident(w) if w == "intersect" => SetOp::Intersect,
                _ => SetOp::Except,
            };
            let all = self.p.eat_word("all");
            let bname = certo_sdl::Ident { name: String::new(), span: bstart };
            let query = Box::new(self.branch(bname, Vec::new())?);
            q.compound.push(SetBranch { op, all, query, span: bstart.to(self.p.prev_span()) });
        }
        if self.p.eat_word("order") {
            self.p.expect_word("by")?;
            loop {
                let expr = self.expr()?;
                let desc = self.p.eat_word("desc");
                if !desc { self.p.eat_word("asc"); }
                q.order_by.push(OrderItem { expr, desc });
                if !self.p.eat(&TokKind::Comma) { break; }
            }
        }
        if self.p.eat_word("limit") { q.limit = Some(self.expr()?); }
        if self.p.eat_word("offset") { q.offset = Some(self.expr()?); }
        q.span = start.to(self.p.prev_span());
        Ok(q)
    }

    /// `with name as (...) [, name as (...)]`
    fn ctes(&mut self) -> PResult<Vec<Cte>> {
        self.p.expect_word("with")?;
        let recursive = self.p.eat_word("recursive");
        let mut out = Vec::new();
        loop {
            let start = self.p.span();
            let name = self.p.ident("a name for the query")?;
            self.p.expect_word("as")?;
            self.p.expect(TokKind::LParen)?;
            let query = self.subquery()?;
            out.push(Cte { recursive, name, query, span: start.to(self.p.prev_span()) });
            if !self.p.eat(&TokKind::Comma) { break; }
        }
        Ok(out)
    }

    /// `from ... [join ...] [where ...] [group by ...] [having ...] select ...`
    fn branch(&mut self, name: certo_sdl::Ident, params: Vec<ParamDecl>) -> PResult<Query> {
        let start = self.p.span();
        self.p.expect_word("from")?;
        let from = self.table_ref(true)?;
        let mut joins = Vec::new();
        while self.word_is("inner") || self.word_is("left") || self.word_is("join") {
            let kind = if self.p.eat_word("left") {
                JoinKind::Left
            } else {
                self.p.eat_word("inner");
                JoinKind::Inner
            };
            self.p.expect_word("join")?;
            let table = self.table_ref(true)?;
            self.p.expect_word("on")?;
            let on = self.expr()?;
            joins.push(Join { kind, table, on });
        }
        let filter = if self.p.eat_word("where") { Some(self.expr()?) } else { None };
        let mut group_by = Vec::new();
        if self.p.eat_word("group") {
            self.p.expect_word("by")?;
            group_by.push(self.expr()?);
            while self.p.eat(&TokKind::Comma) { group_by.push(self.expr()?); }
        }
        let having = if self.p.eat_word("having") { Some(self.expr()?) } else { None };
        self.windows.clear();
        if self.p.eat_word("window") {
            loop {
                let name = self.p.ident("a window name")?;
                self.p.expect_word("as")?;
                self.p.expect(TokKind::LParen)?;
                let spec = self.window_spec()?;
                self.p.expect(TokKind::RParen)?;
                if self.windows.iter().any(|(n, _)| *n == name.name) {
                    return self.fail("QL258", format!("window `{}` is defined twice", name.name), name.span);
                }
                self.windows.push((name.name, spec));
                if !self.p.eat(&TokKind::Comma) { break; }
            }
        }

        self.p.expect_word("select")?;
        let distinct = self.p.eat_word("distinct");
        let mut items = vec![self.select_item()?];
        while self.p.eat(&TokKind::Comma) { items.push(self.select_item()?); }

        Ok(Query {
            name, params, from, joins, filter, group_by, having,
            select: Select { distinct, items },
            order_by: Vec::new(), limit: None, offset: None,
            compound: Vec::new(),
            ctes: Vec::new(),
            span: start.to(self.p.prev_span()),
        })
    }

    /// The inside of `over (...)` / `window w as (...)`: `[partition by ...] [order by ...] [frame]`.
    fn window_spec(&mut self) -> PResult<WindowSpec> {
        let (mut partition_by, mut order_by) = (Vec::new(), Vec::new());
        if self.p.eat_word("partition") {
            self.p.expect_word("by")?;
            partition_by.push(self.expr()?);
            while self.p.eat(&TokKind::Comma) { partition_by.push(self.expr()?); }
        }
        if self.p.eat_word("order") {
            self.p.expect_word("by")?;
            loop {
                let expr = self.expr()?;
                let desc = self.p.eat_word("desc");
                if !desc { self.p.eat_word("asc"); }
                order_by.push(OrderItem { expr, desc });
                if !self.p.eat(&TokKind::Comma) { break; }
            }
        }
        let frame = if self.word_is("rows") || self.word_is("range") || self.word_is("groups") {
            Some(Box::new(self.frame()?))
        } else {
            None
        };
        Ok(WindowSpec { partition_by, order_by, frame })
    }

    /// A subquery after its opening `(` (already consumed); consumes the closing `)`.
    fn subquery(&mut self) -> PResult<Box<Query>> {
        if self.sub_depth >= MAX_SUBQUERY_DEPTH {
            let s = self.p.span();
            return self.fail("QL243", format!("subqueries are nested too deeply (more than {MAX_SUBQUERY_DEPTH} levels)"), s);
        }
        let start = self.p.span();
        self.sub_depth += 1;
        let outer_windows = std::mem::take(&mut self.windows);
        let name = certo_sdl::Ident { name: String::new(), span: start };
        let body = self.query_body(name, Vec::new());
        self.windows = outer_windows;
        self.sub_depth -= 1;
        let mut q = body?;
        let end = self.p.expect(TokKind::RParen)?;
        q.span = start.to(end);
        Ok(Box::new(q))
    }

    /// `( name: type [null], ... )`
    fn param_list(&mut self) -> PResult<Vec<ParamDecl>> {
        self.p.expect(TokKind::LParen)?;
        let mut params = Vec::new();
        if *self.p.peek() != TokKind::RParen {
            loop {
                let pname = self.p.ident("a parameter name")?;
                self.p.expect(TokKind::Colon)?;
                let ty = self.p.type_ref()?;
                let nullable = self.p.eat_word("null");
                params.push(ParamDecl { name: pname, ty, nullable });
                if !self.p.eat(&TokKind::Comma) { break; }
            }
        }
        self.p.expect(TokKind::RParen)?;
        Ok(params)
    }

    /// `col = expr, col = expr, ...`
    fn assignments(&mut self) -> PResult<Vec<Assignment>> {
        let mut out = Vec::new();
        loop {
            let column = self.p.ident("a column name")?;
            if !self.p.eat(&TokKind::Eq) && !self.p.eat(&TokKind::EqEq) {
                return self.p.err("`=`");
            }
            let value = self.expr()?;
            out.push(Assignment { column, value });
            if !self.p.eat(&TokKind::Comma) { break; }
        }
        Ok(out)
    }

    fn mutation(&mut self, kind: MutationKind) -> PResult<Mutation> {
        let start = self.p.span();
        self.p.bump(); // insert | update | delete
        let name = self.p.ident("a name for this statement")?;
        let params = self.param_list()?;
        self.p.expect(TokKind::LBrace)?;
        let named = |q: &Q, at: usize| {
            matches!(q.p.peek_at(at), TokKind::Ident(_)) && matches!(q.p.peek_at(at + 1), TokKind::Ident(w) if w == "as")
        };
        let ctes = if self.word_is("with")
            && (named(self, 1) || (matches!(self.p.peek_at(1), TokKind::Ident(w) if w == "recursive") && named(self, 2)))
        {
            self.ctes()?
        } else {
            Vec::new()
        };

        let table = match kind {
            MutationKind::Insert => { self.p.expect_word("into")?; self.table_ref(false)? }
            MutationKind::Update => self.table_ref(false)?,
            MutationKind::Delete => { self.p.expect_word("from")?; self.table_ref(false)? }
        };
        let (mut assignments, mut insert_columns, mut rows, mut source) = (Vec::new(), Vec::new(), Vec::new(), None);
        if kind == MutationKind::Insert && *self.p.peek() == TokKind::LParen {
            // tabular form: (cols) values (..), (..)   or   (cols) from ... select ...
            self.p.bump();
            insert_columns.push(self.p.ident("a column name")?);
            while self.p.eat(&TokKind::Comma) { insert_columns.push(self.p.ident("a column name")?); }
            self.p.expect(TokKind::RParen)?;
            if self.p.eat_word("values") {
                loop {
                    self.p.expect(TokKind::LParen)?;
                    let mut row = vec![self.expr()?];
                    while self.p.eat(&TokKind::Comma) { row.push(self.expr()?); }
                    self.p.expect(TokKind::RParen)?;
                    rows.push(row);
                    if !self.p.eat(&TokKind::Comma) { break; }
                }
            } else if self.word_is("from") {
                let qstart = self.p.span();
                let name = certo_sdl::Ident { name: String::new(), span: qstart };
                source = Some(Box::new(self.query_body(name, Vec::new())?));
            } else {
                return self.p.err("`values` or `from`");
            }
        } else if kind != MutationKind::Delete {
            self.p.expect_word("set")?;
            assignments = self.assignments()?;
        }

        let mut conflict = None;
        if kind == MutationKind::Insert && self.word_is("on") {
            let cstart = self.p.span();
            self.p.bump();
            self.p.expect_word("conflict")?;
            self.p.expect(TokKind::LParen)?;
            let mut columns = vec![self.p.ident("a column name")?];
            while self.p.eat(&TokKind::Comma) { columns.push(self.p.ident("a column name")?); }
            self.p.expect(TokKind::RParen)?;
            self.p.expect_word("do")?;
            let action = if self.p.eat_word("nothing") {
                ConflictAction::Nothing
            } else {
                self.p.expect_word("update")?;
                self.p.expect_word("set")?;
                ConflictAction::Update(self.assignments()?)
            };
            conflict = Some(Conflict { columns, action, span: cstart.to(self.p.prev_span()) });
        }

        // an update or delete must say which rows: `where ...` or, deliberately, `all rows`
        let (mut filter, mut all_rows) = (None, false);
        if kind != MutationKind::Insert {
            if self.p.eat_word("where") {
                filter = Some(self.expr()?);
            } else if self.word_is("all") {
                self.p.bump();
                self.p.expect_word("rows")?;
                all_rows = true;
            } else {
                return self.p.err("`where <condition>` or `all rows`");
            }
        }

        let mut returning = Vec::new();
        if self.p.eat_word("returning") {
            returning.push(self.select_item()?);
            while self.p.eat(&TokKind::Comma) { returning.push(self.select_item()?); }
        }
        let end = self.p.expect(TokKind::RBrace)?;
        Ok(Mutation { kind, name, params, table, assignments, insert_columns, rows, source, filter, all_rows, conflict, returning, ctes, span: start.to(end) })
    }

    /// A table (or fragment) and its alias. Only `from` and `join` take fragment arguments: after the target of
    /// `insert`, a `(` starts the column list.
    fn table_ref(&mut self, allow_args: bool) -> PResult<TableRef> {
        let table = self.p.ident("a table name")?;
        let mut args = Vec::new();
        if allow_args && self.p.eat(&TokKind::LParen) {
            if *self.p.peek() != TokKind::RParen {
                args.push(self.expr()?);
                while self.p.eat(&TokKind::Comma) { args.push(self.expr()?); }
            }
            self.p.expect(TokKind::RParen)?;
        }
        let explicit = self.p.eat_word("as");
        let alias = match self.p.peek() {
            TokKind::Ident(w) if explicit || !CLAUSE_WORDS.contains(&w.as_str()) => Some(self.p.ident("an alias")?),
            _ if explicit => return self.p.err("an alias"),
            _ => None,
        };
        Ok(TableRef { table, alias, args })
    }

    fn select_item(&mut self) -> PResult<SelectItem> {
        if *self.p.peek() == TokKind::Star {
            return Ok(SelectItem::Star(self.p.bump().span));
        }
        // alias.*
        if matches!(self.p.peek(), TokKind::Ident(_))
            && *self.p.peek_at(1) == TokKind::Dot
            && *self.p.peek_at(2) == TokKind::Star
        {
            let alias = self.p.ident("an alias")?;
            self.p.bump();
            self.p.bump();
            return Ok(SelectItem::SourceStar(alias));
        }
        let expr = self.expr()?;
        let alias = if self.p.eat_word("as") { Some(self.p.ident("a column alias")?) } else { None };
        Ok(SelectItem::Expr { expr, alias })
    }

    // ---- expressions ------------------------------------------------------ //

    fn expr(&mut self) -> PResult<Expr> {
        if self.nesting == 0 { self.nodes = 0; }
        if self.nesting >= MAX_NESTING {
            let s = self.p.span();
            return self.fail("SDL101", format!("expression is nested too deeply (more than {MAX_NESTING} levels)"), s);
        }
        self.nesting += 1;
        let r = self.bp(1);
        self.nesting -= 1;
        r
    }

    fn count(&mut self) -> PResult<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            let s = self.p.span();
            return self.fail("SDL101", format!("expression is too large (more than {MAX_NODES} elements)"), s);
        }
        Ok(())
    }

    fn depth(e: &Expr) -> usize {
        1 + match e {
            Expr::Number(..) | Expr::Decimal(..) | Expr::Str(..) | Expr::Bool(..) | Expr::Null(_)
            | Expr::Column { .. } | Expr::Param(_) | Expr::Exists(..) | Expr::Scalar(..) => 0,
            Expr::Concat(parts, _) => parts.iter().map(Self::depth).max().unwrap_or(0),
            Expr::InQuery { expr, .. } => Self::depth(expr),
            Expr::Binary { lhs, rhs, .. } => Self::depth(lhs).max(Self::depth(rhs)),
            Expr::Not(i, _) | Expr::Paren(i, _) => Self::depth(i),
            Expr::IsNull { expr, .. } => Self::depth(expr),
            Expr::In { expr, list, .. } => Self::depth(expr).max(list.iter().map(Self::depth).max().unwrap_or(0)),
            Expr::Like { expr, pattern, .. } => Self::depth(expr).max(Self::depth(pattern)),
            Expr::Between { expr, low, high, .. } => Self::depth(expr).max(Self::depth(low)).max(Self::depth(high)),
            Expr::Call { args, over, filter, agg_order, .. } => args
                .iter()
                .chain(filter.iter().map(|f| &**f))
                .chain(agg_order.iter().map(|o| &o.expr))
                .chain(over.iter().flat_map(|w| w.partition_by.iter().chain(w.order_by.iter().map(|o| &o.expr))))
                .map(Self::depth)
                .max()
                .unwrap_or(0),
            Expr::Case { whens, otherwise, .. } => whens
                .iter()
                .map(|(w, t)| Self::depth(w).max(Self::depth(t)))
                .chain(otherwise.iter().map(|e| Self::depth(e)))
                .max()
                .unwrap_or(0),
        }
    }

    fn check_depth(&mut self, e: &Expr) -> PResult<()> {
        if Self::depth(e) > MAX_TREE_DEPTH {
            return self.fail("SDL101", format!("expression is too deep (more than {MAX_TREE_DEPTH} levels)"), e.span());
        }
        Ok(())
    }

    fn binop(&self) -> Option<BinaryOp> {
        Some(match self.p.peek() {
            TokKind::EqEq | TokKind::Eq => BinaryOp::Eq,
            TokKind::NotEq => BinaryOp::Ne,
            // `<>` arrives as `<` immediately followed by `>`
            TokKind::Lt if *self.p.peek_at(1) == TokKind::Gt => BinaryOp::Ne,
            TokKind::Lt => BinaryOp::Lt,
            TokKind::Le => BinaryOp::Le,
            TokKind::Gt => BinaryOp::Gt,
            TokKind::Ge => BinaryOp::Ge,
            TokKind::Plus => BinaryOp::Add,
            TokKind::Minus => BinaryOp::Sub,
            TokKind::Star => BinaryOp::Mul,
            TokKind::Slash => BinaryOp::Div,
            TokKind::Ident(s) => match s.as_str() {
                "and" => BinaryOp::And,
                "or" => BinaryOp::Or,
                _ => return None,
            },
            _ => return None,
        })
    }

    fn bp(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = if min <= 3 && self.op_word("not") {
            let start = self.p.span();
            self.count()?;
            self.p.bump();
            // prefix `not` recurses before any node exists to be measured, so guard it directly
            if self.nesting >= MAX_NESTING {
                let s = self.p.span();
                return self.fail("SDL101", format!("expression is nested too deeply (more than {MAX_NESTING} levels)"), s);
            }
            self.nesting += 1;
            let operand = self.bp(3);
            self.nesting -= 1;
            let operand = operand?;
            let span = start.to(operand.span());
            let e = Expr::Not(Box::new(operand), span);
            self.check_depth(&e)?;
            e
        } else {
            self.primary()?
        };
        loop {
            if min <= 4 {
                if self.op_word("is") {
                    self.count()?;
                    self.p.bump();
                    let negated = self.p.eat_word("not");
                    let end = self.p.expect_word("null")?;
                    let span = lhs.span().to(end);
                    lhs = Expr::IsNull { expr: Box::new(lhs), negated, span };
                    self.check_depth(&lhs)?;
                    continue;
                }
                let negated_next = self.op_word("not")
                    && matches!(self.p.peek_at(1), TokKind::Ident(w) if matches!(w.as_str(), "in" | "like" | "between"));
                if self.op_word("in") || (negated_next && matches!(self.p.peek_at(1), TokKind::Ident(w) if w == "in")) {
                    self.count()?;
                    let negated = self.p.eat_word("not");
                    self.p.expect_word("in")?;
                    self.p.expect(TokKind::LParen)?;
                    if self.word_is("from") || self.word_is("with") {
                        let query = self.subquery()?;
                        let span = lhs.span().to(query.span);
                        lhs = Expr::InQuery { expr: Box::new(lhs), query, negated, span };
                        self.check_depth(&lhs)?;
                        continue;
                    }
                    let mut list = vec![self.expr()?];
                    while self.p.eat(&TokKind::Comma) { list.push(self.expr()?); }
                    let end = self.p.expect(TokKind::RParen)?;
                    let span = lhs.span().to(end);
                    lhs = Expr::In { expr: Box::new(lhs), list, negated, span };
                    self.check_depth(&lhs)?;
                    continue;
                }
                if self.op_word("like") || (negated_next && matches!(self.p.peek_at(1), TokKind::Ident(w) if w == "like")) {
                    self.count()?;
                    let negated = self.p.eat_word("not");
                    self.p.expect_word("like")?;
                    let pattern = self.bp(5)?;
                    let span = lhs.span().to(pattern.span());
                    lhs = Expr::Like { expr: Box::new(lhs), pattern: Box::new(pattern), negated, span };
                    self.check_depth(&lhs)?;
                    continue;
                }
                if self.op_word("between") || (negated_next && matches!(self.p.peek_at(1), TokKind::Ident(w) if w == "between")) {
                    self.count()?;
                    let negated = self.p.eat_word("not");
                    self.p.expect_word("between")?;
                    let low = self.bp(5)?;
                    self.p.expect_word("and")?;
                    let high = self.bp(5)?;
                    let span = lhs.span().to(high.span());
                    lhs = Expr::Between { expr: Box::new(lhs), low: Box::new(low), high: Box::new(high), negated, span };
                    self.check_depth(&lhs)?;
                    continue;
                }
            }
            // `||` binds like `+`, left to right; a chain is one concatenation of several parts
            if *self.p.peek() == TokKind::Concat {
                if 5 < min { break; }
                self.count()?;
                self.p.bump();
                let rhs = self.bp(6)?;
                let span = lhs.span().to(rhs.span());
                lhs = match lhs {
                    Expr::Concat(mut parts, _) => {
                        parts.push(rhs);
                        Expr::Concat(parts, span)
                    }
                    other => Expr::Concat(vec![other, rhs], span),
                };
                self.check_depth(&lhs)?;
                continue;
            }
            let Some(op) = self.binop() else { break };
            if op.precedence() < min { break; }
            self.count()?;
            let angle_pair = op == BinaryOp::Ne && *self.p.peek() == TokKind::Lt;
            self.p.bump();
            if angle_pair {
                self.p.bump(); // the `>` of `<>`
            }
            let rhs = self.bp(op.precedence() + 1)?;
            let span = lhs.span().to(rhs.span());
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span };
            self.check_depth(&lhs)?;
        }
        Ok(lhs)
    }

    fn primary(&mut self) -> PResult<Expr> {
        self.count()?;
        let span = self.p.span();
        match self.p.peek().clone() {
            TokKind::Num(n) => {
                self.p.bump();
                match i64::try_from(n) {
                    Ok(v) => Ok(Expr::Number(v, span)),
                    Err(_) => self.fail("SDL002", "number literal is too large", span),
                }
            }
            TokKind::Dec(d) => { self.p.bump(); Ok(Expr::Decimal(d, span)) }
            TokKind::Str(s) => { self.p.bump(); Ok(Expr::Str(s, span)) }
            TokKind::Minus if matches!(self.p.peek_at(1), TokKind::Num(_) | TokKind::Dec(_)) => {
                self.p.bump();
                let lit = span.to(self.p.span());
                match self.p.bump().kind {
                    TokKind::Num(n) => match i64::try_from(-(n as i128)) {
                        Ok(v) => Ok(Expr::Number(v, lit)),
                        Err(_) => self.fail("SDL002", "number literal is too large", lit),
                    },
                    TokKind::Dec(d) => Ok(Expr::Decimal(format!("-{d}"), lit)),
                    _ => unreachable!("peeked a number"),
                }
            }
            // `:name` is a parameter
            TokKind::Colon => {
                self.p.bump();
                let name = self.p.ident("a parameter name")?;
                Ok(Expr::Param(name))
            }
            TokKind::LParen if matches!(self.p.peek_at(1), TokKind::Ident(w) if w == "from" || w == "with") => {
                self.p.bump();
                let query = self.subquery()?;
                let span = span.to(query.span);
                Ok(Expr::Scalar(query, span))
            }
            TokKind::LParen => {
                self.p.bump();
                let e = self.expr()?;
                let end = self.p.expect(TokKind::RParen)?;
                Ok(Expr::Paren(Box::new(e), span.to(end)))
            }
            TokKind::Ident(w) => match w.as_str() {
                "true" | "false" => { self.p.bump(); Ok(Expr::Bool(w == "true", span)) }
                "null" => { self.p.bump(); Ok(Expr::Null(span)) }
                "exists"
                    if *self.p.peek_at(1) == TokKind::LParen
                        && matches!(self.p.peek_at(2), TokKind::Ident(w) if w == "from" || w == "with") =>
                {
                    self.p.bump();
                    self.p.bump();
                    let query = self.subquery()?;
                    let span = span.to(query.span);
                    Ok(Expr::Exists(query, span))
                }
                "case" if !matches!(self.p.peek_at(1), TokKind::Dot | TokKind::LParen) => self.case_expr(span),
                _ => self.name_or_call(),
            },
            _ => self.p.err("an expression"),
        }
    }

    /// `rows|range|groups  <bound>  |  between <bound> and <bound>`
    fn frame(&mut self) -> PResult<Frame> {
        let start_span = self.p.span();
        let units = match self.p.bump().kind {
            TokKind::Ident(w) if w == "rows" => FrameUnits::Rows,
            TokKind::Ident(w) if w == "range" => FrameUnits::Range,
            _ => FrameUnits::Groups,
        };
        let (start, end) = if self.p.eat_word("between") {
            let s = self.frame_bound()?;
            self.p.expect_word("and")?;
            (s, Some(self.frame_bound()?))
        } else {
            (self.frame_bound()?, None)
        };
        Ok(Frame { units, start, end, span: start_span.to(self.p.prev_span()) })
    }

    /// `unbounded preceding | <n> preceding | current row | <n> following | unbounded following`
    fn frame_bound(&mut self) -> PResult<FrameBound> {
        if self.p.eat_word("unbounded") {
            if self.p.eat_word("preceding") {
                return Ok(FrameBound::UnboundedPreceding);
            }
            self.p.expect_word("following")?;
            return Ok(FrameBound::UnboundedFollowing);
        }
        if self.p.eat_word("current") {
            self.p.expect_word("row")?;
            return Ok(FrameBound::CurrentRow);
        }
        let offset = self.expr()?;
        if self.p.eat_word("preceding") {
            Ok(FrameBound::Preceding(offset))
        } else {
            self.p.expect_word("following")?;
            Ok(FrameBound::Following(offset))
        }
    }

    fn case_expr(&mut self, start: Span) -> PResult<Expr> {
        self.p.bump(); // case
        let mut whens = Vec::new();
        while self.p.eat_word("when") {
            let cond = self.expr()?;
            self.p.expect_word("then")?;
            let then = self.expr()?;
            whens.push((cond, then));
        }
        if whens.is_empty() {
            return self.p.err("`when`");
        }
        let otherwise = if self.p.eat_word("else") { Some(Box::new(self.expr()?)) } else { None };
        let end = self.p.expect_word("end")?;
        let e = Expr::Case { whens, otherwise, span: start.to(end) };
        self.check_depth(&e)?;
        Ok(e)
    }

    fn name_or_call(&mut self) -> PResult<Expr> {
        let first = self.p.ident("a name")?;
        if self.p.eat(&TokKind::Dot) {
            let name = self.p.ident("a column name")?;
            return Ok(Expr::Column { qualifier: Some(first), name });
        }
        if !self.p.eat(&TokKind::LParen) {
            return Ok(Expr::Column { qualifier: None, name: first });
        }
        // count(*) / count(distinct x) / f(a, b)
        let mut args = Vec::new();
        let (mut star, mut distinct) = (false, false);
        if self.p.eat(&TokKind::Star) {
            star = true;
        } else if *self.p.peek() != TokKind::RParen {
            distinct = self.p.eat_word("distinct");
            args.push(self.expr()?);
            while self.p.eat(&TokKind::Comma) { args.push(self.expr()?); }
        }
        // `string_agg(x, ", " order by y)`: an `order by` inside the call (only aggregates take one; the checker says so)
        let mut agg_order = Vec::new();
        if self.word_is("order") {
            self.p.bump();
            self.p.expect_word("by")?;
            loop {
                let expr = self.expr()?;
                let desc = self.p.eat_word("desc");
                if !desc { self.p.eat_word("asc"); }
                agg_order.push(OrderItem { expr, desc });
                if !self.p.eat(&TokKind::Comma) { break; }
            }
        }
        let mut end = self.p.expect(TokKind::RParen)?;
        // `agg(...) filter (where cond)`: only the rows that meet the condition feed the aggregate
        let mut filter = None;
        if self.word_is("filter") && *self.p.peek_at(1) == TokKind::LParen {
            self.p.bump();
            self.p.bump();
            self.p.expect_word("where")?;
            let cond = self.expr()?;
            end = self.p.expect(TokKind::RParen)?;
            filter = Some(Box::new(cond));
        }
        // `f(...) over (...)` is a window function
        let mut over = None;
        if self.word_is("over") && *self.p.peek_at(1) == TokKind::LParen {
            self.p.bump();
            self.p.bump();
            let spec = self.window_spec()?;
            end = self.p.expect(TokKind::RParen)?;
            over = Some(spec);
        } else if self.word_is("over") && matches!(self.p.peek_at(1), TokKind::Ident(w) if !CLAUSE_WORDS.contains(&w.as_str())) {
            // `over name`: a window defined by `window name as (...)`
            self.p.bump();
            let name = self.p.ident("a window name")?;
            end = name.span;
            match self.windows.iter().find(|(n, _)| *n == name.name) {
                Some((_, spec)) => over = Some(spec.clone()),
                None => return self.fail("QL258", format!("unknown window `{}` (define it with `window {} as (...)` before `select`)", name.name, name.name), name.span),
            }
        }
        let span = first.span.to(end);
        let e = Expr::Call { func: first, args, star, distinct, filter, agg_order, over, span };
        self.check_depth(&e)?;
        Ok(e)
    }
}
