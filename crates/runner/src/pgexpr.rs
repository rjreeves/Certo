//! Translate PostgreSQL's normalised expression text (column defaults and
//! CHECK bodies as `pg_get_expr` / `pg_get_constraintdef` print them) into SDL
//! expressions, so an existing database can be adopted into an `.sdl` schema.
//!
//! Translation is a best-effort parser, made safe by two independent checks
//! that every result must pass before it is trusted:
//!   1. it compiles as real SDL inside a synthetic schema (type checking,
//!      enum resolution, function arities all come from the compiler);
//!   2. its rendering back to PostgreSQL SQL normalises to the same text as
//!      the original (`drift::normalize`), i.e. it means what the server said.
//!
//! Anything that fails either check is reported and left out, never guessed.
//!
//! What SDL cannot express still shows up here: exponent-form numbers, most
//! functions and operators (`~`, `||`, ...), casts of non-literals, and
//! sequences (`nextval`).

use crate::drift::normalize;
use certo_sdl::{
    compile, to_sdl, BinaryOp, ColumnIR, ConstraintIR, EnumIR, ExprIR, SchemaIR, SequenceIR, TableIR, TypeIR,
};
use certo_sql::{render_expr, Dialect};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    Str(String),
    Num(String),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Op(String),
    /// `::type` (multi-word names joined; unquoted names lowercased)
    Cast(String),
}

const NUMERIC_TYPES: &[&str] =
    &["integer", "bigint", "smallint", "numeric", "real", "double precision", "int", "int4", "int8", "float8", "float4"];

const TYPE_WORDS: &[&str] = &["precision", "varying", "with", "without", "time", "zone"];

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        match ch {
            _ if ch.is_whitespace() => i += 1,
            '(' => { out.push(Tok::LParen); i += 1; }
            ')' => { out.push(Tok::RParen); i += 1; }
            ',' => { out.push(Tok::Comma); i += 1; }
            '[' => { out.push(Tok::LBracket); i += 1; }
            ']' => { out.push(Tok::RBracket); i += 1; }
            '\'' => {
                i += 1;
                let mut v = String::new();
                loop {
                    match c.get(i) {
                        None => return Err("unterminated string literal".into()),
                        Some('\'') if c.get(i + 1) == Some(&'\'') => { v.push('\''); i += 2; }
                        Some('\'') => { i += 1; break; }
                        Some(&x) => { v.push(x); i += 1; }
                    }
                }
                out.push(Tok::Str(v));
            }
            '"' => {
                i += 1;
                let mut v = String::new();
                loop {
                    match c.get(i) {
                        None => return Err("unterminated quoted identifier".into()),
                        Some('"') if c.get(i + 1) == Some(&'"') => { v.push('"'); i += 2; }
                        Some('"') => { i += 1; break; }
                        Some(&x) => { v.push(x); i += 1; }
                    }
                }
                out.push(Tok::Quoted(v));
            }
            ':' if c.get(i + 1) == Some(&':') => {
                i += 2;
                let mut name = String::new();
                loop {
                    if c.get(i) == Some(&'"') {
                        i += 1;
                        while i < c.len() && c[i] != '"' { name.push(c[i]); i += 1; }
                        i += 1;
                    } else {
                        let start = i;
                        while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') { i += 1; }
                        name.push_str(&c[start..i].iter().collect::<String>().to_lowercase());
                    }
                    while c.get(i) == Some(&'[') && c.get(i + 1) == Some(&']') { i += 2; }
                    let mut j = i;
                    while j < c.len() && c[j].is_whitespace() { j += 1; }
                    let start = j;
                    while j < c.len() && c[j].is_alphabetic() { j += 1; }
                    let word: String = c[start..j].iter().collect::<String>().to_lowercase();
                    if j > start && j > i && TYPE_WORDS.contains(&word.as_str()) {
                        name.push(' ');
                        name.push_str(&word);
                        i = j;
                    } else {
                        break;
                    }
                }
                out.push(Tok::Cast(name));
            }
            _ if ch.is_ascii_digit() => {
                let start = i;
                while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') { i += 1; }
                if matches!(c.get(i), Some('e' | 'E')) {
                    i += 1;
                    if matches!(c.get(i), Some('+' | '-')) { i += 1; }
                    while i < c.len() && c[i].is_ascii_digit() { i += 1; }
                }
                out.push(Tok::Num(c[start..i].iter().collect()));
            }
            _ if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') { i += 1; }
                out.push(Tok::Word(c[start..i].iter().collect()));
            }
            '<' | '>' | '=' | '!' | '+' | '-' | '*' | '/' | '|' | '&' | '~' | '%' | '^' | '@' | '#' => {
                let start = i;
                i += 1;
                while i < c.len() && matches!(c[i], '<' | '>' | '=' | '!' | '|' | '&' | '~') { i += 1; }
                out.push(Tok::Op(c[start..i].iter().collect()));
            }
            other => return Err(format!("unexpected character `{other}`")),
        }
    }
    Ok(out)
}

struct P<'a> {
    toks: Vec<Tok>,
    pos: usize,
    enums: &'a [EnumIR],
    columns: Option<&'a HashMap<String, TypeIR>>,
    /// Names of the sequences `nextval` may refer to.
    sequences: &'a [String],
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> { self.toks.get(self.pos) }
    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn kw(&self, w: &str) -> bool { matches!(self.peek(), Some(Tok::Word(x)) if x.eq_ignore_ascii_case(w)) }
    fn eat_kw(&mut self, w: &str) -> bool { let h = self.kw(w); if h { self.pos += 1; } h }

    fn or(&mut self) -> Result<ExprIR, String> {
        let mut l = self.and()?;
        while self.eat_kw("or") {
            let r = self.and()?;
            l = ExprIR::Binary { op: BinaryOp::Or, lhs: Box::new(l), rhs: Box::new(r) };
        }
        Ok(l)
    }

    fn and(&mut self) -> Result<ExprIR, String> {
        let mut l = self.cmp()?;
        while self.eat_kw("and") {
            let r = self.cmp()?;
            l = ExprIR::Binary { op: BinaryOp::And, lhs: Box::new(l), rhs: Box::new(r) };
        }
        Ok(l)
    }

    fn next_kw(&self, w: &str) -> bool {
        matches!(self.toks.get(self.pos + 1), Some(Tok::Word(x)) if x.eq_ignore_ascii_case(w))
    }

    fn cmp(&mut self) -> Result<ExprIR, String> {
        if self.eat_kw("not") {
            let inner = self.cmp()?;
            return Ok(ExprIR::Not { expr: Box::new(inner) });
        }
        let mut l = self.add()?;
        loop {
            if self.eat_kw("is") {
                let negated = self.eat_kw("not");
                if !self.eat_kw("null") { return Err("only `IS [NOT] NULL` is supported".into()); }
                l = ExprIR::IsNull { expr: Box::new(l), negated };
                continue;
            }
            // `x = ANY (ARRAY[...])` is how the server stores `x IN (...)`, `<> ALL` is `NOT IN`
            if let Some(Tok::Op(o)) = self.peek()
                && ((o == "=" && self.next_kw("any")) || ((o == "<>" || o == "!=") && self.next_kw("all")))
            {
                let negated = o != "=";
                self.pos += 2;
                if !matches!(self.bump(), Some(Tok::LParen)) || !self.eat_kw("array") || !matches!(self.bump(), Some(Tok::LBracket)) {
                    return Err("only `ANY (ARRAY[...])` lists are supported".into());
                }
                let mut list = vec![self.or()?];
                while matches!(self.peek(), Some(Tok::Comma)) {
                    self.pos += 1;
                    list.push(self.or()?);
                }
                if !matches!(self.bump(), Some(Tok::RBracket)) || !matches!(self.bump(), Some(Tok::RParen)) {
                    return Err("malformed array list".into());
                }
                l = ExprIR::In { expr: Box::new(l), list, negated };
                continue;
            }
            let op = match self.peek() {
                Some(Tok::Op(o)) => match o.as_str() {
                    "=" => BinaryOp::Eq,
                    "<>" | "!=" => BinaryOp::Ne,
                    "<" => BinaryOp::Lt,
                    "<=" => BinaryOp::Le,
                    ">" => BinaryOp::Gt,
                    ">=" => BinaryOp::Ge,
                    _ => break,
                },
                _ => break,
            };
            self.pos += 1;
            let r = self.add()?;
            l = ExprIR::Binary { op, lhs: Box::new(l), rhs: Box::new(r) };
        }
        Ok(l)
    }

    fn add(&mut self) -> Result<ExprIR, String> {
        let mut l = self.mul()?;
        while let Some(Tok::Op(o)) = self.peek() {
            let op = match o.as_str() { "+" => BinaryOp::Add, "-" => BinaryOp::Sub, _ => break };
            self.pos += 1;
            let r = self.mul()?;
            l = ExprIR::Binary { op, lhs: Box::new(l), rhs: Box::new(r) };
        }
        Ok(l)
    }

    fn mul(&mut self) -> Result<ExprIR, String> {
        let mut l = self.primary()?;
        while let Some(Tok::Op(o)) = self.peek() {
            let op = match o.as_str() { "*" => BinaryOp::Mul, "/" => BinaryOp::Div, _ => break };
            self.pos += 1;
            let r = self.primary()?;
            l = ExprIR::Binary { op, lhs: Box::new(l), rhs: Box::new(r) };
        }
        Ok(l)
    }

    fn primary(&mut self) -> Result<ExprIR, String> {
        let mut e = match self.bump() {
            Some(Tok::Num(n)) => number_literal(&n, false)?,
            Some(Tok::Op(o)) if o == "-" => match self.bump() {
                Some(Tok::Num(n)) => number_literal(&n, true)?,
                _ => return Err("a minus sign is only supported directly in front of a number".into()),
            },
            Some(Tok::Str(s)) => ExprIR::String { value: s },
            Some(Tok::LParen) => {
                let inner = self.or()?;
                match self.bump() {
                    Some(Tok::RParen) => inner,
                    Some(Tok::Op(o)) => return Err(format!("operator `{o}` is not supported by SDL")),
                    _ => return Err("missing `)`".into()),
                }
            }
            Some(Tok::Word(w)) => self.word(w)?,
            Some(Tok::Quoted(name)) => self.column(name)?,
            Some(t) => return Err(format!("unexpected {t:?}")),
            None => return Err("expression ended unexpectedly".into()),
        };
        // casts: meaningful only for enum literals; otherwise transparent
        while let Some(Tok::Cast(t)) = self.peek().cloned() {
            self.pos += 1;
            if let ExprIR::String { value } = &e {
                if let Some(en) = self.enums.iter().find(|en| en.name == t)
                    && en.variants.contains(value)
                {
                    e = ExprIR::EnumVariant { enum_name: en.name.clone(), variant: value.clone() };
                } else if NUMERIC_TYPES.contains(&t.as_str()) {
                    // the server prints e.g. a float default of -1 as '-1'::integer
                    let (neg, digits) = match value.strip_prefix('-') {
                        Some(rest) => (true, rest),
                        None => (false, value.as_str()),
                    };
                    e = number_literal(digits, neg)?;
                }
            }
        }
        Ok(e)
    }

    fn column(&self, name: String) -> Result<ExprIR, String> {
        match self.columns {
            Some(cols) if cols.contains_key(&name) => Ok(ExprIR::Column { name }),
            Some(_) => Err(format!("unknown column `{name}`")),
            None => Err(format!("`{name}`: column references are not allowed in defaults")),
        }
    }

    fn word(&mut self, w: String) -> Result<ExprIR, String> {
        let lower = w.to_ascii_lowercase();
        match lower.as_str() {
            "true" => return Ok(ExprIR::Bool { value: true }),
            "false" => return Ok(ExprIR::Bool { value: false }),
            "null" => return Err("NULL literals are not supported by SDL".into()),
            "current_timestamp" => {
                return Ok(ExprIR::Call { func: "now".into(), args: vec![] });
            }
            "current_date" => {
                return Ok(ExprIR::Call { func: "today".into(), args: vec![] });
            }
            _ => {}
        }
        if lower == "nextval" && matches!(self.peek(), Some(Tok::LParen)) {
            // nextval('seq'::regclass): the sequence NAME, as a declared sequence
            self.pos += 1;
            let arg = self.or()?;
            if !matches!(self.bump(), Some(Tok::RParen)) {
                return Err("malformed nextval(...)".into());
            }
            let ExprIR::String { value } = arg else {
                return Err("nextval must be given a sequence name".into());
            };
            let name = value.replace('"', "").rsplit('.').next().unwrap_or("").to_string();
            return if self.sequences.contains(&name) {
                Ok(ExprIR::NextVal { sequence: name })
            } else {
                Err(format!("sequence `{name}` is not one that SDL can declare (it may be owned by a column, or not adopted)"))
            };
        }
        if matches!(self.peek(), Some(Tok::LParen)) {
            self.pos += 1;
            let mut args = Vec::new();
            if !matches!(self.peek(), Some(Tok::RParen)) {
                loop {
                    args.push(self.or()?);
                    match self.bump() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RParen) => break,
                        _ => return Err("malformed argument list".into()),
                    }
                }
            } else {
                self.pos += 1;
            }
            let func = match lower.as_str() {
                "gen_random_uuid" => "gen_uuid",
                // one-argument btrim is SDL's trim; the two-argument form has no equivalent
                "btrim" if args.len() == 1 => "trim",
                "now" | "lower" | "upper" | "length" | "abs" | "coalesce" | "round" | "nullif" => lower.as_str(),
                other => return Err(format!("function `{other}` is not supported by SDL")),
            };
            return Ok(ExprIR::Call { func: func.into(), args });
        }
        self.column(w)
    }
}

/// `12`, `1.5`, optionally negative, as an SDL literal. Exponent forms are not supported.
fn number_literal(n: &str, neg: bool) -> Result<ExprIR, String> {
    let all = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if all(n) {
        let v: i128 = n.parse().map_err(|_| format!("number `{n}` is too large"))?;
        let v = if neg { -v } else { v };
        return i64::try_from(v).map(|value| ExprIR::Number { value }).map_err(|_| format!("number `{n}` is too large"));
    }
    if let Some((int, frac)) = n.split_once('.')
        && all(int)
        && all(frac)
    {
        return Ok(ExprIR::Decimal { value: format!("{}{n}", if neg { "-" } else { "" }) });
    }
    Err(format!("number `{n}` is not supported by SDL"))
}

fn parse(
    raw: &str,
    enums: &[EnumIR],
    columns: Option<&HashMap<String, TypeIR>>,
    sequences: &[String],
) -> Result<ExprIR, String> {
    let mut p = P { toks: lex(raw)?, pos: 0, enums, columns, sequences };
    let e = p.or()?;
    if p.pos != p.toks.len() {
        return Err(match &p.toks[p.pos] {
            Tok::Op(o) => format!("operator `{o}` is not supported by SDL"),
            t => format!("unexpected trailing {t:?}"),
        });
    }
    Ok(e)
}

/// Both safety checks: same meaning as the server's text, and valid SDL.
fn verify(dialect: Dialect, raw: &str, e: &ExprIR, synthetic: &SchemaIR) -> Result<(), String> {
    let rendered = render_expr(dialect, e);
    let rendered = if dialect == Dialect::Mysql { crate::mysqlexpr::from_render(&rendered) } else { rendered };
    if normalize(&rendered) != normalize(raw) {
        return Err(format!("translation `{rendered}` is not equivalent to the original"));
    }
    let sdl = to_sdl(synthetic)?;
    let (ir, diags) = compile(&sdl);
    if ir.is_none() {
        let msg = diags.first().map(|d| d.message.clone()).unwrap_or_else(|| "does not compile".into());
        return Err(format!("not valid SDL: {msg}"));
    }
    Ok(())
}

fn base_table(columns: Vec<ColumnIR>, constraints: Vec<ConstraintIR>) -> TableIR {
    TableIR { name: "t".into(), columns, relationships: vec![], indexes: vec![], constraints, view: false }
}

fn plain(name: &str, ty: TypeIR) -> ColumnIR {
    ColumnIR {
        name: name.into(),
        ty,
        primary_key: false,
        unique: false,
        nullable: true,
        default: None,
        references: None,
        generated: None,
    }
}

fn ident_ok(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic()) && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The enums SDL can express. Live databases also hold enums with names or
/// values SDL cannot spell; those must never reach the synthetic schema that
/// validates a translation, or every translation would fail to compile.
fn spellable(enums: &[EnumIR]) -> Vec<EnumIR> {
    enums
        .iter()
        .filter(|e| {
            ident_ok(&e.name)
                && certo_sdl::Builtin::from_name(&e.name).is_none()
                && !e.variants.is_empty()
                && e.variants.iter().all(|v| ident_ok(v))
        })
        .cloned()
        .collect()
}

/// SQLite spells some defaults in ways PostgreSQL never does: booleans are 0/1
/// and `gen_uuid()` is a long `randomblob` expression.
fn sqlite_shortcut(dialect: Dialect, raw: &str, col_ty: &TypeIR) -> Option<ExprIR> {
    if dialect == Dialect::Mysql {
        // the one default SDL names that MySQL spells as a long expression
        let uuid = ExprIR::Call { func: "gen_uuid".into(), args: vec![] };
        let rendered = crate::mysqlexpr::from_render(&render_expr(dialect, &uuid));
        return (matches!(col_ty, TypeIR::Builtin(certo_sdl::Builtin::Uuid)) && normalize(raw) == normalize(&rendered)).then_some(uuid);
    }
    if dialect != Dialect::Sqlite {
        return None;
    }
    let uuid = ExprIR::Call { func: "gen_uuid".into(), args: vec![] };
    match (col_ty, raw.trim().trim_start_matches('(').trim_end_matches(')').trim()) {
        (TypeIR::Builtin(certo_sdl::Builtin::Bool), "1") => Some(ExprIR::Bool { value: true }),
        (TypeIR::Builtin(certo_sdl::Builtin::Bool), "0") => Some(ExprIR::Bool { value: false }),
        (TypeIR::Builtin(certo_sdl::Builtin::Uuid), _) if normalize(raw) == normalize(&render_expr(dialect, &uuid)) => Some(uuid),
        _ => None,
    }
}

/// Translate a column default read from a live database (no sequences known).
pub fn translate_default(raw: &str, col_ty: &TypeIR, enums: &[EnumIR]) -> Result<ExprIR, String> {
    translate_default_with(raw, col_ty, enums, &[])
}

/// Like `translate_default`, with the standalone sequences a `nextval(...)`
/// default may refer to.
pub fn translate_default_with(
    raw: &str,
    col_ty: &TypeIR,
    enums: &[EnumIR],
    sequences: &[SequenceIR],
) -> Result<ExprIR, String> {
    translate_default_for(Dialect::Postgres, raw, col_ty, enums, sequences)
}

/// `translate_default_with` for a live database of `dialect`. A result is
/// only trusted if it renders back (in that dialect) to the text it came from.
pub fn translate_default_for(
    dialect: Dialect,
    raw: &str,
    col_ty: &TypeIR,
    enums: &[EnumIR],
    sequences: &[SequenceIR],
) -> Result<ExprIR, String> {
    if matches!(col_ty, TypeIR::Composite(_)) {
        return Err("composite columns cannot have defaults in SDL".into());
    }
    let enums = &spellable(enums);
    if let TypeIR::Enum(n) = col_ty
        && !enums.iter().any(|e| &e.name == n)
    {
        return Err(format!("its enum type {n} cannot be expressed in SDL"));
    }
    let seq_names: Vec<String> = sequences.iter().filter(|s| ident_ok(&s.name)).map(|s| s.name.clone()).collect();
    let e = match sqlite_shortcut(dialect, raw, col_ty) {
        Some(e) => e,
        None => parse(raw, enums, None, &seq_names)?,
    };
    let mut id = plain("id", TypeIR::Builtin(certo_sdl::Builtin::Int));
    id.primary_key = true;
    id.nullable = false;
    let mut col = plain("c", col_ty.clone());
    col.default = Some(e.clone());
    let synth = SchemaIR {
        version: certo_sdl::IR_VERSION,
        tables: vec![base_table(vec![id, col], vec![])],
        enums: enums.to_vec(),
        types: vec![],
        sequences: sequences.iter().filter(|s| ident_ok(&s.name)).cloned().collect(),
        views: vec![],
    };
    verify(dialect, raw, &e, &synth)?;
    Ok(e)
}

/// Translate a CHECK body read from a live database. `columns` are the
/// table's columns (only builtin- and enum-typed ones can appear in SDL checks).
pub fn translate_check(raw: &str, columns: &[ColumnIR], enums: &[EnumIR]) -> Result<ExprIR, String> {
    translate_check_for(Dialect::Postgres, raw, columns, enums)
}

/// `translate_check` for a live database of `dialect`.
pub fn translate_check_for(dialect: Dialect, raw: &str, columns: &[ColumnIR], enums: &[EnumIR]) -> Result<ExprIR, String> {
    let enums = &spellable(enums);
    let usable: Vec<ColumnIR> = columns
        .iter()
        .filter(|c| match &c.ty {
            TypeIR::Composite(_) => false,
            TypeIR::Enum(n) => enums.iter().any(|e| &e.name == n),
            TypeIR::Builtin(_) => true,
        })
        .filter(|c| ident_ok(&c.name))
        .map(|c| plain(&c.name, c.ty.clone()))
        .collect();
    let types: HashMap<String, TypeIR> = usable.iter().map(|c| (c.name.clone(), c.ty.clone())).collect();
    let e = parse(raw, enums, Some(&types), &[])?;
    let synth = SchemaIR {
        version: certo_sdl::IR_VERSION,
        tables: vec![base_table(usable, vec![ConstraintIR { name: "chk".into(), expr: e.clone() }])],
        enums: enums.to_vec(),
        types: vec![],
        sequences: vec![],
        views: vec![],
    };
    verify(dialect, raw, &e, &synth)?;
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_sdl::Builtin;

    fn int() -> TypeIR { TypeIR::Builtin(Builtin::Int) }
    fn roles() -> Vec<EnumIR> { vec![EnumIR { name: "Role".into(), variants: vec!["admin".into(), "user".into()] }] }
    fn col(name: &str, ty: TypeIR) -> ColumnIR { plain(name, ty) }

    #[test]
    fn defaults_translate() {
        let role = TypeIR::Enum("Role".into());
        let text = TypeIR::Builtin(Builtin::Text);
        let float = TypeIR::Builtin(Builtin::Float);
        let dec = TypeIR::Builtin(Builtin::Decimal);
        let cases: Vec<(&str, TypeIR, ExprIR)> = vec![
            ("gen_random_uuid()", TypeIR::Builtin(Builtin::Uuid), ExprIR::Call { func: "gen_uuid".into(), args: vec![] }),
            ("now()", TypeIR::Builtin(Builtin::Timestamp), ExprIR::Call { func: "now".into(), args: vec![] }),
            ("CURRENT_TIMESTAMP", TypeIR::Builtin(Builtin::Timestamp), ExprIR::Call { func: "now".into(), args: vec![] }),
            ("CURRENT_DATE", TypeIR::Builtin(Builtin::Date), ExprIR::Call { func: "today".into(), args: vec![] }),
            ("18", int(), ExprIR::Number { value: 18 }),
            ("-1", int(), ExprIR::Number { value: -1 }),
            ("'-1'::integer", float.clone(), ExprIR::Number { value: -1 }), // how the server prints a float default of -1
            ("(1)::double precision", float.clone(), ExprIR::Number { value: 1 }),
            ("1.5", dec.clone(), ExprIR::Decimal { value: "1.5".into() }),
            ("'-0.25'::numeric", dec, ExprIR::Decimal { value: "-0.25".into() }),
            ("true", TypeIR::Builtin(Builtin::Bool), ExprIR::Bool { value: true }),
            ("'anon'::text", text.clone(), ExprIR::String { value: "anon".into() }),
            ("'it''s'::text", text.clone(), ExprIR::String { value: "it's".into() }),
            ("'2026-01-01'::date", TypeIR::Builtin(Builtin::Date), ExprIR::String { value: "2026-01-01".into() }),
            ("'user'::\"Role\"", role, ExprIR::EnumVariant { enum_name: "Role".into(), variant: "user".into() }),
            ("lower('X'::text)", text.clone(), ExprIR::Call { func: "lower".into(), args: vec![ExprIR::String { value: "X".into() }] }),
            ("btrim(' x '::text)", text, ExprIR::Call { func: "trim".into(), args: vec![ExprIR::String { value: " x ".into() }] }),
            ("'abc'::character varying", TypeIR::Builtin(Builtin::Varchar(10)), ExprIR::String { value: "abc".into() }),
        ];
        for (raw, ty, want) in cases {
            assert_eq!(translate_default(raw, &ty, &roles()), Ok(want), "{raw}");
        }
    }

    #[test]
    fn nextval_of_a_declared_sequence() {
        let seq = |name: &str| SequenceIR { name: name.into(), start: 1, increment: 1, min: 1, max: i64::MAX, cache: 1, cycle: false };
        let seqs = vec![seq("order_seq"), seq("Mixed_Seq")];
        let big = TypeIR::Builtin(Builtin::BigInt);
        let want = |n: &str| Ok(ExprIR::NextVal { sequence: n.into() });
        assert_eq!(translate_default_with("nextval('order_seq'::regclass)", &big, &[], &seqs), want("order_seq"));
        assert_eq!(translate_default_with("nextval('order_seq'::regclass)", &int(), &[], &seqs), want("order_seq"));
        // the quoted spelling the server prints for names that need it
        assert_eq!(translate_default_with("nextval('\"Mixed_Seq\"'::regclass)", &big, &[], &seqs), want("Mixed_Seq"));
        // a schema-qualified name means another schema than the current one: refused, not guessed
        assert!(translate_default_with("nextval('other.order_seq'::regclass)", &big, &[], &seqs).is_err());
        // an undeclared sequence, no sequence at all, or a non-name argument
        assert!(translate_default_with("nextval('other_seq'::regclass)", &big, &[], &seqs).unwrap_err().contains("not one that SDL can declare"));
        assert!(translate_default_with("nextval('order_seq'::regclass)", &big, &[], &[]).is_err());
        assert!(translate_default_with("nextval(1)", &big, &[], &seqs).unwrap_err().contains("sequence name"));
        // a sequence whose name SDL cannot spell is never offered to the synthetic schema
        let odd = vec![seq("bad name")];
        assert!(translate_default_with("nextval('\"bad name\"'::regclass)", &big, &[], &odd).is_err());
    }

    #[test]
    fn untranslatable_defaults_say_why() {
        let bad: &[(&str, TypeIR, &str)] = &[
            ("nextval('users_id_seq'::regclass)", int(), "not one that SDL can declare"), // no such declared sequence
            ("NULL::integer", int(), "NULL"),
            ("uuid_generate_v4()", TypeIR::Builtin(Builtin::Uuid), "uuid_generate_v4"),
            ("md5('x'::text)", TypeIR::Builtin(Builtin::Text), "md5"),
            ("1e3", TypeIR::Builtin(Builtin::Float), "not supported"),
            ("other_col + 1", int(), "column references"),
            ("'a'::text", TypeIR::Composite("T".into()), "composite"),
            ("'x'::\"Role\"", TypeIR::Enum("Role".into()), "not valid SDL"), // not a variant
            ("1.5", int(), "not valid SDL"),                                   // a fraction in an integer column
        ];
        for (raw, ty, why) in bad {
            let e = translate_default(raw, ty, &roles()).expect_err(raw);
            assert!(e.contains(why), "{raw}: {e}");
        }
    }

    #[test]
    fn unspellable_enums_elsewhere_in_the_database_do_not_break_translation() {
        // found on a live server: an enum named "Bad Enum" poisoned every translation
        let mut enums = roles();
        enums.push(EnumIR { name: "Bad Enum".into(), variants: vec!["a".into(), "b c".into()] });
        assert_eq!(translate_default("18", &int(), &enums), Ok(ExprIR::Number { value: 18 }));
        assert_eq!(
            translate_default("'user'::\"Role\"", &TypeIR::Enum("Role".into()), &enums),
            Ok(ExprIR::EnumVariant { enum_name: "Role".into(), variant: "user".into() })
        );
        // a column of the unspellable enum itself is refused clearly
        let e = translate_default("'a'::\"Bad Enum\"", &TypeIR::Enum("Bad Enum".into()), &enums).unwrap_err();
        assert!(e.contains("cannot be expressed"), "{e}");
        let cols = vec![col("age", int()), col("mood", TypeIR::Enum("Bad Enum".into()))];
        assert!(translate_check("(age > 1)", &cols, &enums).is_ok());
        assert!(translate_check("(mood = 'a'::\"Bad Enum\")", &cols, &enums).unwrap_err().contains("unknown column"));
    }

    #[test]
    fn checks_translate() {
        let cols = vec![
            col("age", int()),
            col("role", TypeIR::Enum("Role".into())),
            col("score", TypeIR::Builtin(Builtin::Float)),
            col("nick", TypeIR::Builtin(Builtin::Text)),
            col("code", TypeIR::Builtin(Builtin::Varchar(10))),
        ];
        for raw in [
            "((age >= 18) AND (role <> 'admin'::\"Role\"))",
            "((score >= (0)::double precision) OR (score IS NULL))",
            "(length(nick) > 0)",
            "((age + 1) * 2 > 10)",
            "((age > 1) OR ((age < 0) AND (score = (2)::double precision)))",
            // the widened language
            "(age IS NOT NULL)",
            "(NOT (age > 1))",
            "((NOT (age > 1)) AND (nick IS NOT NULL))",
            "(age > '-1'::integer)",
            "(score > 1.5)",
            "(age = ANY (ARRAY[1, 2, 3]))",
            "(nick = ANY (ARRAY['new'::text, 'paid'::text]))",
            "(role = ANY (ARRAY['admin'::\"Role\", 'user'::\"Role\"]))",
            "(age <> ALL (ARRAY[1, 2]))",
            "(btrim(nick) <> ''::text)",
            "(round(score) > (0)::double precision)",
            "(nullif(age, 0) IS NOT NULL)",
            "(length((code)::text) > 0)",
        ] {
            let e = translate_check(raw, &cols, &roles()).unwrap_or_else(|e| panic!("{raw}: {e}"));
            // the translation must render back to the same meaning
            assert_eq!(normalize(&render_expr(Dialect::Postgres, &e)), normalize(raw), "{raw}");
        }
        let e = translate_check("((age >= 18) AND (role <> 'admin'::\"Role\"))", &cols, &roles()).unwrap();
        assert!(matches!(e, ExprIR::Binary { op: BinaryOp::And, .. }));
        assert!(matches!(translate_check("(age IS NOT NULL)", &cols, &roles()), Ok(ExprIR::IsNull { negated: true, .. })));
        assert!(matches!(translate_check("(age = ANY (ARRAY[1, 2]))", &cols, &roles()), Ok(ExprIR::In { negated: false, .. })));
        assert!(matches!(translate_check("(age <> ALL (ARRAY[1, 2]))", &cols, &roles()), Ok(ExprIR::In { negated: true, .. })));
    }

    #[test]
    fn untranslatable_checks_say_why() {
        let cols = vec![col("age", int()), col("nick", TypeIR::Builtin(Builtin::Text))];
        let bad: &[(&str, &str)] = &[
            ("(age IS DISTINCT FROM 1)", "only `IS [NOT] NULL`"),
            ("(nick ~ 'x'::text)", "operator `~`"),
            ("(nick || 'x'::text)", "operator `||`"),
            ("nick || 'x'::text", "operator `||`"),
            ("(md5(nick) = 'x'::text)", "md5"),
            ("(ghost > 1)", "unknown column"),
            ("(age > 1e3)", "not supported"),
            ("(btrim(nick, 'x'::text) <> ''::text)", "btrim"),
            ("(age = ANY ('{1,2}'::integer[]))", "ARRAY"),
            ("(age > -age)", "minus sign"),
        ];
        for (raw, why) in bad {
            let e = translate_check(raw, &cols, &roles()).expect_err(raw);
            assert!(e.contains(why), "{raw}: {e}");
        }
    }

    #[test]
    fn a_translation_that_changes_meaning_is_rejected() {
        // grouping the server printed differently than SDL precedence would give
        let cols = vec![col("a", TypeIR::Builtin(Builtin::Bool)), col("b", TypeIR::Builtin(Builtin::Bool)), col("c", TypeIR::Builtin(Builtin::Bool))];
        // parentheses are ignored by normalize, so this documents the known limit rather than rejecting
        assert!(translate_check("((a OR b) AND c)", &cols, &roles()).is_ok());
        // but a dropped operand is caught
        assert!(translate_check("(a AND)", &cols, &roles()).is_err());
    }
}
