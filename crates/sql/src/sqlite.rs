//! SQLite lowering (3.35+: `RETURNING`, `DROP COLUMN`, `RENAME COLUMN`).
//!
//! SQLite has dynamic typing and a small `ALTER TABLE`, so this adapter differs
//! from PostgreSQL in three ways:
//!
//! * Types map to SQLite affinities with a distinguishing declared name (`uuid`
//!   is `UUID TEXT`, `date` is `DATE TEXT`, `bool` is `BOOLEAN`, `bytes` is
//!   `BLOB`). An enum is `ENUM_<name> TEXT` with an inline
//!   `CHECK (col IN (...))`.
//! * Anything `ALTER TABLE` cannot do (change a column, add or drop a foreign
//!   key or CHECK, add a NOT NULL/UNIQUE/computed column, change an enum's
//!   values) is done by rebuilding the table: create the new shape, copy the
//!   rows, drop the old table, rename, recreate indexes. The new shape is the
//!   table as it is in the *new* schema, so lowering needs both schemas
//!   ([`crate::Schemas`]); one rebuild absorbs every structural change to that
//!   table in the plan. Rebuilds run with `PRAGMA foreign_keys = OFF` around
//!   the transaction (SQLite ignores the pragma inside one).
//! * A table created in the plan is written in one piece, foreign keys and
//!   CHECKs included, rather than created and then altered.
//!
//! Auto-generated values: SQLite can only generate an integer primary key
//! (`INTEGER PRIMARY KEY AUTOINCREMENT`); `serial` and both identity forms map
//! to that, and anything else is refused. Sequences and composite types do not
//! exist in SQLite and are refused.

use crate::{unsupported, Batch, LowerError, Schemas};
use certo_mdl::{action_sql, MigrationPlan, Op};
use certo_sdl::{BinaryOp, Builtin, ColumnIR, ExprIR, TableIR, TypeIR};
use std::collections::{HashMap, HashSet};

pub(crate) fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub(crate) fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The declared type name. SQLite derives the column's affinity from it (a name
/// containing `TEXT` has TEXT affinity) and keeps the name as written, so the
/// extra word (`UUID TEXT`, `DATE TEXT`, `ENUM_Role TEXT`, ...) is what lets
/// introspection tell the types apart again.
pub(crate) fn ty(t: &TypeIR) -> String {
    match t {
        TypeIR::Builtin(b) => match b {
            Builtin::Text => "TEXT".into(),
            Builtin::Uuid => "UUID TEXT".into(),
            Builtin::Json => "JSON TEXT".into(),
            Builtin::SmallInt => "SMALLINT".into(),
            Builtin::Int => "INTEGER".into(),
            Builtin::BigInt => "BIGINT".into(),
            Builtin::Decimal => "NUMERIC".into(),
            Builtin::Real => "REAL".into(),
            Builtin::Float => "DOUBLE".into(),
            Builtin::Bool => "BOOLEAN".into(),
            Builtin::Timestamp => "TIMESTAMPTZ TEXT".into(),
            Builtin::TimestampNaive => "TIMESTAMP TEXT".into(),
            Builtin::Date => "DATE TEXT".into(),
            Builtin::Bytes => "BLOB".into(),
            Builtin::Varchar(n) => format!("VARCHAR({n})"),
            Builtin::Char(n) => format!("CHAR({n})"),
            Builtin::Numeric(p, s) => format!("NUMERIC({p},{s})"),
        },
        TypeIR::Enum(n) => format!("ENUM_{n} TEXT"),
        TypeIR::Composite(_) => "TEXT".into(),
    }
}

const UUID_V4: &str = "(lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' || \
substr(lower(hex(randomblob(2))), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || \
substr(lower(hex(randomblob(2))), 2) || '-' || lower(hex(randomblob(6))))";

/// SQL for an expression. `Err` names what SQLite cannot express.
pub(crate) fn expr(e: &ExprIR) -> Result<String, String> {
    Ok(match e {
        ExprIR::Raw { sql } => sql.clone(),
        ExprIR::Number { value } if *value < 0 => format!("({value})"),
        ExprIR::Number { value } => value.to_string(),
        ExprIR::Decimal { value } if value.starts_with('-') => format!("({value})"),
        ExprIR::Decimal { value } => value.clone(),
        ExprIR::IsNull { expr: inner, negated } => {
            format!("({} IS {}NULL)", expr(inner)?, if *negated { "NOT " } else { "" })
        }
        ExprIR::Not { expr: inner } => format!("(NOT {})", expr(inner)?),
        ExprIR::In { expr: inner, list, negated } => {
            let items = list.iter().map(expr).collect::<Result<Vec<_>, _>>()?;
            format!("({} {}IN ({}))", expr(inner)?, if *negated { "NOT " } else { "" }, items.join(", "))
        }
        ExprIR::String { value } => lit(value),
        ExprIR::Bool { value } => if *value { "1" } else { "0" }.to_string(),
        ExprIR::Column { name } => q(name),
        ExprIR::EnumVariant { variant, .. } => lit(variant),
        ExprIR::NextVal { .. } => return Err("SQLite has no sequences (`nextval`)".into()),
        ExprIR::Call { func, args } => match func.as_str() {
            "today" => "CURRENT_DATE".to_string(),
            "now" => "CURRENT_TIMESTAMP".to_string(),
            "gen_uuid" => UUID_V4.to_string(),
            "is_null" => format!("({} IS NULL)", expr(&args[0])?),
            other => {
                let args = args.iter().map(expr).collect::<Result<Vec<_>, _>>()?;
                format!("{other}({})", args.join(", "))
            }
        },
        ExprIR::Binary { op, lhs, rhs } => {
            let o = match op {
                BinaryOp::Eq => "=",
                BinaryOp::Ne => "<>",
                BinaryOp::Lt => "<",
                BinaryOp::Le => "<=",
                BinaryOp::Gt => ">",
                BinaryOp::Ge => ">=",
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::And => "AND",
                BinaryOp::Or => "OR",
            };
            format!("({} {o} {})", expr(lhs)?, expr(rhs)?)
        }
    })
}

/// Is this a plain literal (the only kind of default `ADD COLUMN` accepts)?
fn is_const(e: &ExprIR) -> bool {
    matches!(
        e,
        ExprIR::Number { .. } | ExprIR::Decimal { .. } | ExprIR::String { .. } | ExprIR::Bool { .. } | ExprIR::EnumVariant { .. }
    )
}

fn default_sql(e: &ExprIR) -> Result<String, String> {
    if matches!(e, ExprIR::Call { func, .. } if func == "now" || func == "today") || is_const(e) {
        return expr(e);
    }
    Ok(format!("({})", expr(e)?))
}

fn check_expr(e: &ExprIR) -> Result<String, String> {
    match e {
        ExprIR::Binary { .. } => expr(e),
        _ => Ok(format!("({})", expr(e)?)),
    }
}

/// Everything a table definition needs from the schema.
struct Ctx<'a> {
    schemas: &'a Schemas<'a>,
}

impl Ctx<'_> {
    fn variants(&self, enum_name: &str) -> Result<&[String], String> {
        self.schemas
            .new
            .enums
            .iter()
            .find(|e| e.name == enum_name)
            .map(|e| e.variants.as_slice())
            .ok_or_else(|| format!("enum `{enum_name}` is not in the new schema"))
    }

    /// `"col" TYPE [PRIMARY KEY AUTOINCREMENT] [NOT NULL] [DEFAULT ..] [CHECK ..]`.
    fn column_def(&self, table: &TableIR, c: &ColumnIR) -> Result<String, String> {
        if matches!(c.ty, TypeIR::Composite(_)) {
            return Err(format!("column `{}` has a composite type, which SQLite does not have", c.name));
        }
        let mut s = format!("{} ", q(&c.name));
        let auto = c.generated.is_some();
        if auto {
            let only_pk = c.primary_key && table.columns.iter().filter(|x| x.primary_key).count() == 1;
            let int = matches!(c.ty, TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt));
            if !(only_pk && int) {
                return Err(format!(
                    "column `{}` is generated, but SQLite can only generate a single-column integer primary key",
                    c.name
                ));
            }
            // must be spelled exactly INTEGER to alias the rowid
            s.push_str("INTEGER PRIMARY KEY AUTOINCREMENT");
            return Ok(s);
        }
        s.push_str(&ty(&c.ty));
        if !c.nullable {
            s.push_str(" NOT NULL");
        }
        if let Some(d) = &c.default {
            s.push_str(&format!(" DEFAULT {}", default_sql(d)?));
        }
        if let TypeIR::Enum(name) = &c.ty {
            let vs: Vec<String> = self.variants(name)?.iter().map(|v| lit(v)).collect();
            s.push_str(&format!(" CHECK ({} IN ({}))", q(&c.name), vs.join(", ")));
        }
        Ok(s)
    }

    /// `CREATE TABLE name (...)` for `t`'s final shape (foreign keys and CHECKs included).
    fn create_table(&self, t: &TableIR, name: &str) -> Result<String, String> {
        let mut parts = Vec::new();
        for c in &t.columns {
            parts.push(self.column_def(t, c)?);
        }
        let auto_pk = t.columns.iter().any(|c| c.generated.is_some() && c.primary_key);
        let pks: Vec<String> = t.columns.iter().filter(|c| c.primary_key).map(|c| q(&c.name)).collect();
        if !pks.is_empty() && !auto_pk {
            parts.push(format!("CONSTRAINT {} PRIMARY KEY ({})", q(&format!("{}_pkey", t.name)), pks.join(", ")));
        }
        for c in t.columns.iter().filter(|c| c.unique && !c.primary_key) {
            parts.push(format!("CONSTRAINT {} UNIQUE ({})", q(&format!("{}_{}_key", t.name, c.name)), q(&c.name)));
        }
        for c in &t.columns {
            if let Some(r) = &c.references {
                let mut s = format!(
                    "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                    q(&format!("fk_{}_{}", t.name, c.name)), q(&c.name), q(&r.table), q(&r.column)
                );
                for (kw, a) in [("DELETE", r.on_delete), ("UPDATE", r.on_update)] {
                    if a != certo_sdl::ReferentialAction::NoAction {
                        s.push_str(&format!(" ON {kw} {}", action_sql(a)));
                    }
                }
                parts.push(s);
            }
        }
        for k in &t.constraints {
            parts.push(format!("CONSTRAINT {} CHECK {}", q(&k.name), check_expr(&k.expr)?));
        }
        let body: Vec<String> = parts.iter().map(|p| format!("    {p}")).collect();
        Ok(format!("CREATE TABLE {} (\n{}\n);", q(name), body.join(",\n")))
    }
}

fn create_index(table: &str, index: &certo_sdl::IndexIR) -> String {
    let cols: Vec<_> = index.columns.iter().map(|c| q(c)).collect();
    format!("CREATE INDEX IF NOT EXISTS {} ON {} ({});", q(&index.name), q(table), cols.join(", "))
}

struct Lowerer<'a> {
    plan: &'a MigrationPlan,
    ctx: Ctx<'a>,
    out: Vec<String>,
    created: HashSet<String>,
    rebuilt: HashSet<String>,
    /// The columns each table physically has at this point of the script.
    phys: HashMap<String, Vec<String>>,
    used_rebuild: bool,
}

pub(crate) fn lower_batches(plan: &MigrationPlan, schemas: &Schemas) -> Result<Vec<Batch>, LowerError> {
    let mut l = Lowerer {
        plan,
        ctx: Ctx { schemas },
        out: Vec::new(),
        created: HashSet::new(),
        rebuilt: HashSet::new(),
        phys: HashMap::new(),
        used_rebuild: false,
    };
    for op in &plan.ops {
        l.op(op)?;
    }
    let mut batches = Vec::new();
    if l.used_rebuild {
        batches.push(Batch { transactional: false, statements: vec!["PRAGMA foreign_keys = OFF;".into()] });
    }
    if !l.out.is_empty() {
        batches.push(Batch { transactional: true, statements: std::mem::take(&mut l.out) });
    }
    if l.used_rebuild {
        batches.push(Batch { transactional: false, statements: vec!["PRAGMA foreign_keys = ON;".into()] });
    }
    Ok(batches)
}

impl Lowerer<'_> {
    fn physical(&mut self, table: &str) -> Vec<String> {
        let old = self.ctx.schemas.old;
        self.phys
            .entry(table.to_string())
            .or_insert_with(|| old.table(table).map(|t| t.columns.iter().map(|c| c.name.clone()).collect()).unwrap_or_default())
            .clone()
    }

    /// Is this table's shape already going to be (or has it been) written from the new schema?
    fn settled(&self, table: &str) -> bool {
        self.created.contains(table) || self.rebuilt.contains(table)
    }

    fn map<T>(op: &Op, r: Result<T, String>) -> Result<T, LowerError> {
        r.map_err(|reason| unsupported(op, reason))
    }

    fn op(&mut self, op: &Op) -> Result<(), LowerError> {
        match op {
            // enums are TEXT + CHECK; their DDL lives in the columns that use them
            Op::CreateEnum { .. } | Op::DropEnum { .. } => {}
            Op::AddEnumVariant { name, .. } | Op::RemoveEnumVariant { name, .. } => self.rebuild_enum_users(op, name)?,
            Op::RecreateEnum { name, mappings, columns, .. } => {
                for m in mappings {
                    for c in columns {
                        self.out.push(format!(
                            "UPDATE {} SET {col} = {} WHERE {col} = {};",
                            q(&c.table), lit(&m.to), lit(&m.from), col = q(&c.column.name)
                        ));
                    }
                }
                self.rebuild_enum_users(op, name)?;
            }

            Op::CreateSequence { .. } | Op::AlterSequence { .. } | Op::DropSequence { .. } => {
                return Err(unsupported(op, "SQLite has no sequences; use an auto-generated integer primary key"));
            }
            Op::CreateType { .. } | Op::AlterType { .. } | Op::DropType { .. } => {
                return Err(unsupported(op, "SQLite has no composite types"));
            }

            Op::CreateTable { definition } => {
                let t = self.ctx.schemas.new.table(&definition.name).unwrap_or(definition);
                let sql = Self::map(op, self.ctx.create_table(t, &t.name))?;
                self.out.push(sql);
                self.created.insert(t.name.clone());
                self.phys.insert(t.name.clone(), t.columns.iter().map(|c| c.name.clone()).collect());
            }
            Op::DropTable { name } => {
                self.out.push(format!("DROP TABLE {};", q(name)));
                self.phys.remove(name);
                self.created.remove(name);
                self.rebuilt.remove(name);
            }

            Op::CreateView { definition: v } => {
                let cols: Vec<String> = v.columns.iter().map(|c| q(c)).collect();
                let mut s = format!("CREATE VIEW {} AS SELECT {} FROM {}", q(&v.name), cols.join(", "), q(&v.from));
                if let Some(f) = &v.filter {
                    s.push_str(&format!(" WHERE {}", Self::map(op, expr(f))?));
                }
                self.out.push(s + ";");
            }
            Op::DropView { name } => self.out.push(format!("DROP VIEW {};", q(name))),

            Op::AddColumn { table, column } => {
                if self.settled(table) {
                    return Ok(());
                }
                let native = !column.primary_key
                    && !column.unique
                    && column.generated.is_none()
                    && match &column.default {
                        Some(d) => is_const(d),
                        None => column.nullable,
                    };
                if native {
                    let t = self.ctx.schemas.new.table(table).cloned();
                    let holder = t.unwrap_or_else(|| TableIR {
                        name: table.clone(),
                        columns: vec![column.clone()],
                        relationships: vec![],
                        indexes: vec![],
                        constraints: vec![],
                        view: false,
                    });
                    let def = Self::map(op, self.ctx.column_def(&holder, column))?;
                    self.out.push(format!("ALTER TABLE {} ADD COLUMN {def};", q(table)));
                    let mut cols = self.physical(table);
                    cols.push(column.name.clone());
                    self.phys.insert(table.clone(), cols);
                } else {
                    self.rebuild(op, table)?;
                }
            }
            Op::AlterColumn { table, .. }
            | Op::DropColumn { table, .. }
            | Op::AddForeignKey { table, .. }
            | Op::DropForeignKey { table, .. }
            | Op::AddConstraint { table, .. }
            | Op::DropConstraint { table, .. } => {
                if !self.settled(table) {
                    self.rebuild(op, table)?;
                }
            }
            Op::Backfill { table, column, value } => {
                let v = Self::map(op, expr(value))?;
                self.out.push(format!("UPDATE {} SET {} = {v} WHERE {} IS NULL;", q(table), q(column), q(column)));
                if !self.settled(table) {
                    self.rebuild(op, table)?; // the NOT NULL half of the backfill
                }
            }

            // relationships are query-level sugar over foreign keys
            Op::AddRelationship { .. } | Op::DropRelationship { .. } => {}

            Op::CreateIndex { table, index } => self.out.push(create_index(table, index)),
            Op::DropIndex { name, .. } => self.out.push(format!("DROP INDEX IF EXISTS {};", q(name))),

            Op::RenameTable { from, to, .. } => {
                self.out.push(format!("ALTER TABLE {} RENAME TO {};", q(from), q(to)));
                let cols = self.physical(from);
                self.phys.remove(from);
                self.phys.insert(to.clone(), cols);
                for set in [&mut self.created, &mut self.rebuilt] {
                    if set.remove(from) {
                        set.insert(to.clone());
                    }
                }
            }
            Op::RenameColumn { table, from, to, .. } => {
                self.out.push(format!("ALTER TABLE {} RENAME COLUMN {} TO {};", q(table), q(from), q(to)));
                let mut cols = self.physical(table);
                for c in &mut cols {
                    if c == from {
                        *c = to.clone();
                    }
                }
                self.phys.insert(table.clone(), cols);
            }
            Op::DataUpdate { table, set, filter } => {
                let mut assigns = Vec::new();
                for a in set {
                    assigns.push(format!("{} = {}", q(&a.column), Self::map(op, expr(&a.value))?));
                }
                let mut s = format!("UPDATE {} SET {}", q(table), assigns.join(", "));
                if let Some(f) = filter {
                    s.push_str(&format!(" WHERE {}", Self::map(op, expr(f))?));
                }
                s.push(';');
                self.out.push(s);
            }
            Op::RawSql { dialect, sql } => {
                if dialect.as_deref().is_none_or(|d| d == "sqlite") {
                    let s = sql.trim();
                    self.out.push(if s.ends_with(';') { s.to_string() } else { format!("{s};") });
                }
            }
        }
        Ok(())
    }

    /// Tables that already exist and use `enum_name` need their CHECK rewritten.
    fn rebuild_enum_users(&mut self, op: &Op, enum_name: &str) -> Result<(), LowerError> {
        let tables: Vec<String> = self
            .ctx
            .schemas
            .new
            .tables
            .iter()
            .filter(|t| t.columns.iter().any(|c| matches!(&c.ty, TypeIR::Enum(n) if n == enum_name)))
            .map(|t| t.name.clone())
            .collect();
        for t in tables {
            if !self.settled(&t) {
                self.rebuild(op, &t)?;
            }
        }
        Ok(())
    }

    /// Replace `table` with its shape in the new schema, keeping its rows.
    fn rebuild(&mut self, op: &Op, table: &str) -> Result<(), LowerError> {
        let new_t = self
            .ctx
            .schemas
            .new
            .table(table)
            .ok_or_else(|| unsupported(op, format!("cannot rebuild `{table}`: it is not in the new schema")))?
            .clone();
        // rows of enum variants that are going away must be remapped before the new CHECK sees them
        for other in &self.plan.ops {
            if let Op::RecreateEnum { mappings, columns, .. } = other {
                for m in mappings {
                    for c in columns.iter().filter(|c| c.table == table) {
                        self.out.push(format!(
                            "UPDATE {} SET {col} = {} WHERE {col} = {};",
                            q(table), lit(&m.to), lit(&m.from), col = q(&c.column.name)
                        ));
                    }
                }
            }
        }

        let tmp = format!("__certo_new_{table}");
        let phys = self.physical(table);
        let create = Self::map(op, self.ctx.create_table(&new_t, &tmp))?;
        self.out.push(create);

        let mut cols = Vec::new();
        let mut selects = Vec::new();
        for c in &new_t.columns {
            if !phys.contains(&c.name) {
                continue; // a new column: its default (or generation) fills it
            }
            cols.push(q(&c.name));
            selects.push(match (&c.default, c.nullable) {
                // tightening to NOT NULL: existing NULLs take the column's default
                (Some(d), false) => format!("COALESCE({}, {})", q(&c.name), Self::map(op, expr(d))?),
                _ => q(&c.name),
            });
        }
        if !cols.is_empty() {
            self.out.push(format!(
                "INSERT INTO {} ({}) SELECT {} FROM {};",
                q(&tmp), cols.join(", "), selects.join(", "), q(table)
            ));
        }
        self.out.push(format!("DROP TABLE {};", q(table)));
        self.out.push(format!("ALTER TABLE {} RENAME TO {};", q(&tmp), q(table)));
        for i in &new_t.indexes {
            self.out.push(create_index(table, i));
        }
        self.rebuilt.insert(table.to_string());
        self.phys.insert(table.to_string(), new_t.columns.iter().map(|c| c.name.clone()).collect());
        self.used_rebuild = true;
        Ok(())
    }
}
