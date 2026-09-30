//! The MDL language: hand-written directives that steer how the difference
//! between two schemas is migrated. Grammar: `docs/ebnf.md` (MDL section).
//!
//! ```text
//! rename table users -> accounts
//! rename column users.nick -> username
//! remap Role.guest -> user
//! backfill users.display_name = username
//! before { update users set age = 18 where is_null(age) }
//! after  { sql postgres "ANALYZE users" }
//! ```
//!
//! Keywords are contextual, like SDL's. Expressions are SDL expressions.

use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use certo_sdl::{Expr, Ident, PResult, Parser, TokKind};

#[derive(Debug, Clone, PartialEq)]
pub struct MdlFile {
    pub directives: Vec<Directive>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Directive {
    RenameTable { from: Ident, to: Ident, span: Span },
    /// `table` is the table's name in the *old* schema.
    RenameColumn { table: Ident, from: Ident, to: Ident, span: Span },
    /// Removing enum variant `from` maps existing rows to `to`.
    Remap { enum_name: Ident, from: Ident, to: Ident, span: Span },
    /// Fill a new (or newly NOT NULL) column before it is made NOT NULL.
    Backfill { table: Ident, column: Ident, value: Expr, span: Span },
    Block { when: When, steps: Vec<Step>, span: Span },
}

impl Directive {
    pub fn span(&self) -> Span {
        match self {
            Directive::RenameTable { span, .. }
            | Directive::RenameColumn { span, .. }
            | Directive::Remap { span, .. }
            | Directive::Backfill { span, .. }
            | Directive::Block { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Runs before any schema change; sees the old schema.
    Before,
    /// Runs after every schema change; sees the new schema.
    After,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Update { table: Ident, set: Vec<Assign>, filter: Option<Expr>, span: Span },
    /// Raw SQL for one dialect (or, with no dialect, for all of them).
    Sql { dialect: Option<Ident>, sql: String, span: Span },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
    pub column: Ident,
    pub value: Expr,
}

const DIRECTIVE_WORDS: &[&str] = &["rename", "remap", "backfill", "before", "after"];

/// Parse an MDL file. The AST holds every directive that parsed cleanly.
pub fn parse(src: &str) -> (MdlFile, Vec<Diagnostic>) {
    let mut p = Parser::new(src);
    let mut directives = Vec::new();
    while !p.at_eof() {
        let start = p.pos();
        match directive(&mut p) {
            Ok(d) => directives.push(d),
            Err(()) => {
                if p.pos() == start { p.bump(); }
                p.recover_to(DIRECTIVE_WORDS);
            }
        }
    }
    (MdlFile { directives }, p.into_diagnostics())
}

fn directive(p: &mut Parser) -> PResult<Directive> {
    let TokKind::Ident(word) = p.peek().clone() else {
        return p.err("a directive (`rename`, `remap`, `backfill`, `before` or `after`)");
    };
    let start = p.span();
    match word.as_str() {
        "rename" => {
            p.bump();
            if p.eat_word("table") {
                let from = p.ident("a table name")?;
                p.expect(TokKind::Arrow)?;
                let to = p.ident("the new table name")?;
                Ok(Directive::RenameTable { span: start.to(to.span), from, to })
            } else if p.eat_word("column") {
                let table = p.ident("a table name")?;
                p.expect(TokKind::Dot)?;
                let from = p.ident("a column name")?;
                p.expect(TokKind::Arrow)?;
                let to = p.ident("the new column name")?;
                Ok(Directive::RenameColumn { span: start.to(to.span), table, from, to })
            } else {
                p.err("`table` or `column`")
            }
        }
        "remap" => {
            p.bump();
            let enum_name = p.ident("an enum name")?;
            p.expect(TokKind::Dot)?;
            let from = p.ident("the variant being removed")?;
            p.expect(TokKind::Arrow)?;
            let to = p.ident("the variant to move its rows to")?;
            Ok(Directive::Remap { span: start.to(to.span), enum_name, from, to })
        }
        "backfill" => {
            p.bump();
            let table = p.ident("a table name")?;
            p.expect(TokKind::Dot)?;
            let column = p.ident("a column name")?;
            p.expect(TokKind::Eq)?;
            let value = p.expr()?;
            let span = start.to(value.span());
            Ok(Directive::Backfill { table, column, value, span })
        }
        "before" | "after" => {
            let when = if word == "before" { When::Before } else { When::After };
            p.bump();
            p.expect(TokKind::LBrace)?;
            let mut steps = Vec::new();
            while *p.peek() != TokKind::RBrace {
                steps.push(step(p)?);
            }
            let end = p.expect(TokKind::RBrace)?;
            Ok(Directive::Block { when, steps, span: start.to(end) })
        }
        _ => p.err("a directive (`rename`, `remap`, `backfill`, `before` or `after`)"),
    }
}

fn step(p: &mut Parser) -> PResult<Step> {
    let start = p.span();
    if p.eat_word("update") {
        let table = p.ident("a table name")?;
        p.expect_word("set")?;
        let mut set = vec![assign(p)?];
        while p.eat(&TokKind::Comma) {
            set.push(assign(p)?);
        }
        let filter = if p.eat_word("where") { Some(p.expr()?) } else { None };
        let end = filter.as_ref().map_or(set.last().map(|a| a.value.span()).unwrap_or(start), |f| f.span());
        Ok(Step::Update { table, set, filter, span: start.to(end) })
    } else if p.eat_word("sql") {
        let dialect = if matches!(p.peek(), TokKind::Ident(_)) { Some(p.ident("a dialect")?) } else { None };
        let (sql, span) = p.string_lit("a SQL string")?;
        Ok(Step::Sql { dialect, sql, span: start.to(span) })
    } else {
        p.err("`update` or `sql`")
    }
}

fn assign(p: &mut Parser) -> PResult<Assign> {
    let column = p.ident("a column name")?;
    p.expect(TokKind::Eq)?;
    let value = p.expr()?;
    Ok(Assign { column, value })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> MdlFile {
        let (f, d) = parse(src);
        assert!(d.is_empty(), "unexpected diagnostics: {d:?}");
        f
    }

    #[test]
    fn every_directive_parses() {
        let f = ok("// a migration
            rename table users -> accounts
            rename column users.nick -> username
            remap Role.guest -> user
            backfill users.display = username
            before { update users set age = 18, active = true where is_null(age) }
            after { sql postgres \"ANALYZE users\"  sql \"SELECT 1\" }");
        assert_eq!(f.directives.len(), 6);
        assert!(matches!(&f.directives[0], Directive::RenameTable { from, to, .. } if from.name == "users" && to.name == "accounts"));
        assert!(matches!(&f.directives[1], Directive::RenameColumn { table, from, to, .. }
            if table.name == "users" && from.name == "nick" && to.name == "username"));
        assert!(matches!(&f.directives[2], Directive::Remap { enum_name, from, to, .. }
            if enum_name.name == "Role" && from.name == "guest" && to.name == "user"));
        assert!(matches!(&f.directives[3], Directive::Backfill { column, .. } if column.name == "display"));
        let Directive::Block { when, steps, .. } = &f.directives[4] else { panic!() };
        assert_eq!(*when, When::Before);
        let Step::Update { set, filter, .. } = &steps[0] else { panic!() };
        assert_eq!(set.len(), 2);
        assert!(filter.is_some());
        let Directive::Block { when, steps, .. } = &f.directives[5] else { panic!() };
        assert_eq!(*when, When::After);
        assert!(matches!(&steps[0], Step::Sql { dialect: Some(d), sql, .. } if d.name == "postgres" && sql == "ANALYZE users"));
        assert!(matches!(&steps[1], Step::Sql { dialect: None, .. }));
    }

    #[test]
    fn empty_file_and_update_without_where() {
        assert!(ok("").directives.is_empty());
        let f = ok("after { update t set a = 1 }");
        let Directive::Block { steps, .. } = &f.directives[0] else { panic!() };
        assert!(matches!(&steps[0], Step::Update { filter: None, .. }));
    }

    #[test]
    fn errors_recover_at_the_next_directive() {
        let (f, d) = parse("rename nonsense\nremap Role.a -> b\nbackfill t.c\nrename table a -> b");
        assert!(d.len() >= 2, "{d:?}");
        let kinds: Vec<_> = f.directives.iter().map(|d| matches!(d, Directive::Remap { .. } | Directive::RenameTable { .. })).collect();
        assert_eq!(kinds, [true, true]);
    }

    #[test]
    fn junk_does_not_loop_or_crash() {
        for s in ["}", "{ } ", "@@@", "rename", "before {", "before { update", "\"str\"", "backfill t.c = ("] {
            let (_, d) = parse(s);
            assert!(!d.is_empty(), "{s:?} should be an error");
        }
    }
}
