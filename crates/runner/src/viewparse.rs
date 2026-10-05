//! Read a view back from a database as an SDL view: `view name on table (columns) where condition`.
//!
//! A database gives a view back as the SQL it keeps (PostgreSQL's `pg_get_viewdef`, SQLite's stored `CREATE VIEW`, MySQL's
//! `information_schema.views`), written its own way. Only the shape SDL can say is read: a plain
//! `SELECT column, column FROM table [WHERE condition]` over one table, with no aliases, joins, grouping, ordering or limits.
//! Anything else is not an SDL view and is reported (a note when importing, a changed definition in drift).
//!
//! A reading is only trusted when it passes two checks: the condition goes through the same translator as a CHECK
//! constraint (so it is valid SDL), and the SQL certo would write for the view normalises to the same text as what the
//! database kept. Otherwise nothing is guessed.

use crate::drift::normalize;
use crate::mysqlexpr;
use certo_mdl::{MigrationPlan, Op, PLAN_VERSION};
use certo_sdl::{EnumIR, SchemaIR, TableIR, ViewIR};
use certo_sql::Dialect;

/// Keywords that mean the view is more than a plain select of one table.
const NOT_PLAIN: &[&str] = &[
    "join", "group", "having", "order", "limit", "offset", "union", "intersect", "except", "distinct", "with", "window", "fetch", "for",
];

/// One word or punctuation mark of the SQL, with quoted names and strings kept whole.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    /// A bare word (a keyword, a name, a number).
    Word(String),
    /// A quoted name (`"x"` or `` `x` ``), unquoted.
    Name(String),
    /// A string literal, as written (quotes included).
    Str(String),
    Sym(char),
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < c.len() {
        match c[i] {
            ch if ch.is_whitespace() => i += 1,
            q @ ('"' | '`') => {
                let mut v = String::new();
                i += 1;
                loop {
                    match c.get(i) {
                        None => return Err("unterminated name".into()),
                        Some(&x) if x == q && c.get(i + 1) == Some(&q) => {
                            v.push(q);
                            i += 2;
                        }
                        Some(&x) if x == q => {
                            i += 1;
                            break;
                        }
                        Some(&x) => {
                            v.push(x);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Name(v));
            }
            '\'' => {
                let start = i;
                i += 1;
                loop {
                    match c.get(i) {
                        None => return Err("unterminated string".into()),
                        Some('\'') if c.get(i + 1) == Some(&'\'') => i += 2,
                        Some('\\') if i + 1 < c.len() => i += 2,
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
                out.push(Tok::Str(c[start..i].iter().collect()));
            }
            ch if ch.is_alphanumeric() || ch == '_' || ch == '$' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                    i += 1;
                }
                out.push(Tok::Word(c[start..i].iter().collect()));
            }
            ch => {
                out.push(Tok::Sym(ch));
                i += 1;
            }
        }
    }
    Ok(out)
}

fn is_kw(t: &Tok, kw: &str) -> bool {
    matches!(t, Tok::Word(w) if w.eq_ignore_ascii_case(kw))
}

/// The select, whatever surrounds it (SQLite keeps the whole `CREATE VIEW ... AS`).
fn select_text(definition: &str) -> &str {
    let d = definition.trim().trim_end_matches(';').trim();
    let upper = d.to_uppercase();
    if upper.starts_with("CREATE") {
        // after the first top-level ` AS `
        if let Some(at) = upper.find(" AS ") {
            return d[at + 4..].trim();
        }
    }
    d
}

/// `a.b.c` -> `c`: the qualifiers of a name (table, database) are dropped, in every dotted chain outside strings.
fn drop_qualifiers(toks: &[Tok]) -> Vec<Tok> {
    let name_like = |t: &Tok| matches!(t, Tok::Name(_)) || matches!(t, Tok::Word(w) if !w.chars().next().is_some_and(|c| c.is_ascii_digit()));
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        // a chain: name . name [. name]
        if name_like(&toks[i]) {
            let mut j = i;
            while j + 2 < toks.len() && toks[j + 1] == Tok::Sym('.') && name_like(&toks[j + 2]) {
                j += 2;
            }
            out.push(toks[j].clone());
            i = j + 1;
        } else {
            out.push(toks[i].clone());
            i += 1;
        }
    }
    out
}

/// Text again, for the condition (the translator reads text).
fn text(toks: &[Tok]) -> String {
    let mut s = String::new();
    for t in toks {
        match t {
            Tok::Word(w) => {
                if s.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    s.push(' ');
                }
                s.push_str(w);
            }
            Tok::Name(n) => {
                if s.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    s.push(' ');
                }
                s.push_str(&format!("\"{}\"", n.replace('"', "\"\"")));
            }
            Tok::Str(v) => {
                if s.chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    s.push(' ');
                }
                s.push_str(v);
            }
            Tok::Sym(c) => {
                // keep operators apart from the word before them
                let operator = |c: char| matches!(c, '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '|' | '&' | '%');
                if operator(*c) && s.chars().last().is_some_and(|l| !l.is_whitespace() && !operator(l)) {
                    s.push(' ');
                }
                s.push(*c);
            }
        }
    }
    s
}

/// The name a token stands for (a bare word is lowercased only if the database folds, which we cannot know: kept as is).
fn ident(t: &Tok) -> Option<String> {
    match t {
        Tok::Word(w) if !w.chars().next().is_some_and(|c| c.is_ascii_digit()) => Some(w.clone()),
        Tok::Name(n) => Some(n.clone()),
        _ => None,
    }
}

/// Read `definition` (as `dialect` keeps it) as the SDL view `name`, over `tables`.
pub fn parse_view(dialect: Dialect, name: &str, definition: &str, tables: &[TableIR], enums: &[EnumIR]) -> Result<ViewIR, String> {
    let body = select_text(definition);
    // MySQL's own spelling first (backticks, introducers, `\\'` escapes), so that everything below reads one dialect
    let flavoured;
    let body = if dialect == Dialect::Mysql {
        flavoured = mysqlexpr::from_render(body);
        flavoured.as_str()
    } else {
        body
    };
    let toks = drop_qualifiers(&lex(body)?);

    // the clauses, at the top level
    let mut depth = 0i32;
    let (mut from_at, mut where_at) = (None, None);
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Sym('(') => depth += 1,
            Tok::Sym(')') => depth -= 1,
            _ if depth == 0 => {
                if i > 0 && is_kw(t, "from") && from_at.is_none() {
                    from_at = Some(i);
                } else if is_kw(t, "where") && where_at.is_none() {
                    where_at = Some(i);
                } else if let Tok::Word(w) = t
                    && NOT_PLAIN.iter().any(|k| w.eq_ignore_ascii_case(k))
                {
                    return Err(format!("it uses `{}`: only a plain select of one table's columns can be an SDL view", w.to_lowercase()));
                }
            }
            _ => {}
        }
    }
    if !toks.first().is_some_and(|t| is_kw(t, "select")) {
        return Err("it is not a plain select".into());
    }
    let (Some(from_at), end_select) = (from_at, where_at.unwrap_or(toks.len())) else {
        return Err("it has no FROM".into());
    };
    if from_at >= end_select {
        return Err("it is not a plain select of one table".into());
    }

    // columns: `column` or `column AS column`
    let mut columns: Vec<String> = Vec::new();
    for item in toks[1..from_at].split(|t| *t == Tok::Sym(',')) {
        let col = match item {
            [c] => ident(c),
            [c, kw, a] if is_kw(kw, "as") && ident(c).is_some() && ident(c) == ident(a) => ident(c),
            _ => None,
        };
        columns.push(col.ok_or("it selects something other than plain columns")?);
    }
    // the table
    let from = match &toks[from_at + 1..end_select] {
        [t] => ident(t).ok_or("it reads something other than a table")?,
        _ => return Err("it reads more than one table, or aliases its table".into()),
    };
    let table = tables.iter().find(|t| t.name == from).ok_or_else(|| format!("its table `{from}` is not in the schema"))?;
    if let Some(missing) = columns.iter().find(|c| table.column(c).is_none()) {
        return Err(format!("`{from}.{missing}` is not a column the schema has"));
    }
    let mut kept = format!("SELECT {} FROM \"{from}\"", columns.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", "));
    let filter = match where_at {
        None => None,
        Some(w) => {
            let cond = text(&toks[w + 1..]);
            kept.push_str(&format!(" WHERE {cond}"));
            Some(crate::pgexpr::translate_check_for(dialect, &cond, &table.columns, enums)?)
        }
    };
    let view = ViewIR { name: name.to_string(), from, columns, filter };

    // trusted only if certo's own SQL for it says the same as the database's
    let ours = own_select(dialect, &view)?;
    // (`kept` is the database's own condition, with the plain columns and table it was read as: a select that is
    // not just those would have been refused above)
    let (a, b) = (clean(dialect, &ours), clean(dialect, &kept));
    if normalize(&a) != normalize(&b) {
        return Err(format!("its SQL (`{}`) is not what an SDL view of that shape would be", b.trim()));
    }
    Ok(view)
}

fn clean(dialect: Dialect, sql: &str) -> String {
    if dialect == Dialect::Mysql { mysqlexpr::from_render(sql) } else { sql.to_string() }
}

/// The select certo writes for `view` in `dialect`.
fn own_select(dialect: Dialect, view: &ViewIR) -> Result<String, String> {
    let plan = MigrationPlan { version: PLAN_VERSION, ops: vec![Op::CreateView { definition: view.clone() }] };
    let empty = SchemaIR::empty();
    let stmts = certo_sql::lower_with(&plan, dialect, certo_sql::Schemas { old: &empty, new: &empty }).map_err(|e| e.to_string())?;
    let sql = stmts.first().ok_or("no SQL")?;
    let at = sql.find(" AS SELECT").ok_or("unexpected SQL")?;
    Ok(sql[at + 4..].trim_end_matches(';').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_sdl::compile;

    fn schema() -> SchemaIR {
        compile("enum Role { admin, user }
                 table users { id: serial primary key  email: text not null  age: int  role: Role not null  name: varchar(20) }")
            .0
            .unwrap()
    }

    fn read(dialect: Dialect, sql: &str) -> Result<ViewIR, String> {
        let s = schema();
        parse_view(dialect, "v", sql, &s.tables, &s.enums)
    }

    #[test]
    fn what_each_database_keeps_reads_back() {
        // PostgreSQL's pg_get_viewdef (pretty)
        let v = read(Dialect::Postgres, " SELECT id,\n    email\n   FROM users\n  WHERE age >= 18 AND role <> 'admin'::\"Role\";").unwrap();
        assert_eq!((v.from.as_str(), v.columns.as_slice()), ("users", ["id".to_string(), "email".to_string()].as_slice()));
        assert!(v.filter.is_some());
        let v = read(Dialect::Postgres, " SELECT email,\n    age\n   FROM users;").unwrap();
        assert!(v.filter.is_none() && v.columns == ["email", "age"]);
        // SQLite's own text, as certo wrote it
        let v = read(Dialect::Sqlite, "CREATE VIEW \"v\" AS SELECT \"id\", \"email\" FROM \"users\" WHERE (\"age\" >= 18)").unwrap();
        assert_eq!(v.columns, ["id", "email"]);
        // MySQL's information_schema text
        let v = read(
            Dialect::Mysql,
            "select `d`.`users`.`id` AS `id`,`d`.`users`.`email` AS `email` from `d`.`users` where ((`d`.`users`.`age` >= 18) and (`d`.`users`.`role` <> 'admin'))",
        )
        .unwrap();
        assert_eq!(v.columns, ["id", "email"]);
        assert!(v.filter.is_some());
    }

    #[test]
    fn anything_more_than_a_plain_select_is_not_an_sdl_view() {
        for (sql, why) in [
            ("SELECT id FROM users ORDER BY id", "order"),
            ("SELECT id FROM users LIMIT 5", "limit"),
            ("SELECT id FROM users GROUP BY id", "group"),
            ("SELECT DISTINCT id FROM users", "distinct"),
            ("SELECT id FROM users u JOIN users v ON u.id = v.id", "aliases"),
            ("SELECT id + 1 FROM users", "plain columns"),
            ("SELECT id AS x FROM users", "plain columns"),
            ("SELECT id FROM nope", "not in the schema"),
            ("SELECT ghost FROM users", "not a column"),
        ] {
            let e = read(Dialect::Postgres, sql).unwrap_err();
            assert!(e.contains(why) || e.contains("only a plain select") || e.contains("more than one table"), "{sql}: {e}");
        }
    }
}
