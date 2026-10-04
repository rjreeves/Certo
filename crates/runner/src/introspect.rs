//! Read a live PostgreSQL schema back into a `SchemaIR`.
//!
//! The mapping is deliberately conservative: anything SDL cannot express
//! (varchar(n), multi-column unique or foreign keys, partial/expression/unique
//! indexes, identity columns, ...) is left out of the IR and reported in
//! `notes`, never guessed. Defaults and CHECK bodies come back as
//! `ExprIR::Raw` (server-normalised SQL text); `drift::align` reconciles them
//! with the expected expressions.
//!
//! Scope: the connection's current schema. The history table is skipped.

use certo_sdl::{
    Builtin, ColumnIR, CompositeIR, ConstraintIR, EnumIR, ExprIR, FieldIR, ForeignKeyIR, Generation, IndexIR,
    ReferentialAction, SchemaIR, SequenceIR, TableIR, TypeIR,
};
use postgres::Client;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const HISTORY_TABLE: &str = "_certo_migrations";

#[derive(Debug, Clone, PartialEq)]
pub struct LiveSchema {
    pub ir: SchemaIR,
    /// Live objects SDL cannot represent, in words.
    pub notes: Vec<String>,
}

/// `character varying(255)`, `character(3)`, `numeric(10,2)`.
fn map_parameterised(t: &str) -> Option<Builtin> {
    if let Some(n) = t.strip_prefix("character varying(").and_then(|r| r.strip_suffix(')')) {
        let n: u32 = n.parse().ok()?;
        return (1..=10_485_760).contains(&n).then_some(Builtin::Varchar(n));
    }
    if let Some(n) = t.strip_prefix("character(").and_then(|r| r.strip_suffix(')')) {
        let n: u32 = n.parse().ok()?;
        return (1..=10_485_760).contains(&n).then_some(Builtin::Char(n));
    }
    if let Some(args) = t.strip_prefix("numeric(").and_then(|r| r.strip_suffix(')')) {
        let (p, s) = match args.split_once(',') {
            Some((p, s)) => (p.trim().parse::<u16>().ok()?, s.trim().parse::<u16>().ok()?),
            None => (args.trim().parse::<u16>().ok()?, 0),
        };
        return ((1..=1000).contains(&p) && s <= p).then_some(Builtin::Numeric(p, s));
    }
    None
}

fn map_builtin(t: &str) -> Option<Builtin> {
    Some(match t {
        "text" => Builtin::Text,
        "smallint" => Builtin::SmallInt,
        "real" => Builtin::Real,
        "timestamp without time zone" => Builtin::TimestampNaive,
        "integer" => Builtin::Int,
        "bigint" => Builtin::BigInt,
        "numeric" => Builtin::Decimal,
        "double precision" => Builtin::Float,
        "boolean" => Builtin::Bool,
        "uuid" => Builtin::Uuid,
        "timestamp with time zone" => Builtin::Timestamp,
        "date" => Builtin::Date,
        "jsonb" => Builtin::Json,
        "bytea" => Builtin::Bytes,
        _ => return None,
    })
}

fn action(code: &str) -> Option<ReferentialAction> {
    Some(match code {
        "a" => ReferentialAction::NoAction,
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        _ => return None,
    })
}

/// Last path component of a possibly schema-qualified, possibly quoted name.
fn seq_base(s: &str) -> String {
    s.replace('"', "").rsplit('.').next().unwrap_or("").to_string()
}

/// Is `default` (as `pg_get_expr` prints it) a `nextval` of exactly `seq`?
fn is_nextval_of(default: &str, seq: &str) -> bool {
    let Some(rest) = default.trim().strip_prefix("nextval(") else { return false };
    let inner = rest.strip_suffix(')').unwrap_or(rest);
    let literal = inner.split("::").next().unwrap_or("").trim().trim_matches('\'');
    seq_base(literal) == seq_base(seq)
}

struct Types {
    enums: HashSet<String>,
    composites: HashSet<String>,
}

impl Types {
    /// `format_type` output -> SDL type, or `None` if SDL cannot express it.
    fn map(&self, formatted: &str) -> Option<TypeIR> {
        let t = formatted.trim_matches('"');
        if let Some(b) = map_builtin(t).or_else(|| map_parameterised(t)) {
            return Some(TypeIR::Builtin(b));
        }
        if self.enums.contains(t) {
            return Some(TypeIR::Enum(t.to_string()));
        }
        if self.composites.contains(t) {
            return Some(TypeIR::Composite(t.to_string()));
        }
        None
    }
}

pub fn introspect(client: &mut Client) -> Result<LiveSchema, postgres::Error> {
    let mut notes: Vec<String> = Vec::new();
    // certo's own tables are not part of the schema
    let reserved: Vec<String> = vec![HISTORY_TABLE.to_string(), crate::journal::LOG_TABLE.to_string()];

    // ---- enums ------------------------------------------------------------
    let mut enums: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in client.query(
        "SELECT t.typname::text, e.enumlabel::text
         FROM pg_type t
         JOIN pg_enum e ON e.enumtypid = t.oid
         JOIN pg_namespace n ON n.oid = t.typnamespace
         WHERE n.nspname = current_schema()
         ORDER BY t.typname, e.enumsortorder",
        &[],
    )? {
        enums.entry(r.get(0)).or_default().push(r.get(1));
    }

    // ---- composite type names (fields need every type name known first) ----
    let composite_rows = client.query(
        "SELECT t.typname::text, a.attname::text, format_type(a.atttypid, a.atttypmod)
         FROM pg_type t
         JOIN pg_class c ON c.oid = t.typrelid AND c.relkind = 'c'
         JOIN pg_namespace n ON n.oid = t.typnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
         WHERE n.nspname = current_schema()
         ORDER BY t.typname, a.attnum",
        &[],
    )?;
    let types = Types {
        enums: enums.keys().cloned().collect(),
        composites: composite_rows.iter().map(|r| r.get::<_, String>(0)).collect(),
    };
    let mut composites: BTreeMap<String, Vec<FieldIR>> = BTreeMap::new();
    for r in &composite_rows {
        let (tname, fname, ftype): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let fields = composites.entry(tname.clone()).or_default();
        match types.map(&ftype) {
            Some(ty) => fields.push(FieldIR { name: fname, ty }),
            None => notes.push(format!("type {tname}: field `{fname}` has type {ftype}, which SDL cannot express")),
        }
    }

    // ---- tables and columns ----------------------------------------------
    let mut tables: BTreeMap<String, Vec<ColumnIR>> = BTreeMap::new();
    let mut defaults: HashMap<(String, String), String> = HashMap::new();
    for r in client.query(
        "SELECT c.relname::text, a.attname::text, format_type(a.atttypid, a.atttypmod),
                a.attnotnull, pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, a.attgenerated::text,
                pg_get_serial_sequence(quote_ident(c.relname), a.attname)
         FROM pg_class c
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
         LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum
         WHERE n.nspname = current_schema() AND c.relkind IN ('r', 'p') AND c.relname::text <> ALL($1)
         ORDER BY c.relname, a.attnum",
        &[&reserved],
    )? {
        let (table, col, ftype): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let (not_null, mut default): (bool, Option<String>) = (r.get(3), r.get(4));
        let (identity, generated): (String, String) = (r.get(5), r.get(6));
        let serial_seq: Option<String> = r.get(7);
        let cols = tables.entry(table.clone()).or_default();
        let Some(ty) = types.map(&ftype) else {
            notes.push(format!("column {table}.{col} has type {ftype}, which SDL cannot express; ignored"));
            continue;
        };
        // identity, or serial: an integer column whose default is nextval of the sequence it owns
        let integer = matches!(ty, TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt));
        let mut generation = match identity.as_str() {
            "a" => Some(Generation::Always),
            "d" => Some(Generation::ByDefault),
            _ => None,
        };
        if generation.is_none()
            && integer
            && let (Some(seq), Some(d)) = (&serial_seq, &default)
            && is_nextval_of(d, seq)
        {
            generation = Some(Generation::Serial);
            default = None; // implied by the generation
        }
        if !generated.is_empty() {
            notes.push(format!("column {table}.{col} is a generated column; SDL has no equivalent"));
        }
        if let Some(d) = default {
            defaults.insert((table.clone(), col.clone()), d);
        }
        cols.push(ColumnIR {
            name: col,
            ty,
            primary_key: false,
            unique: false,
            nullable: !not_null,
            default: None,
            references: None,
            generated: generation,
        });
    }
    for ((table, col), d) in defaults {
        if let Some(c) = tables.get_mut(&table).and_then(|cs| cs.iter_mut().find(|c| c.name == col)) {
            c.default = Some(ExprIR::Raw { sql: d });
        }
    }

    // ---- constraints ------------------------------------------------------
    let mut checks: BTreeMap<String, Vec<ConstraintIR>> = BTreeMap::new();
    for r in client.query(
        "SELECT t.relname::text, con.conname::text, con.contype::text,
                array(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord)
                      JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum ORDER BY k.ord),
                ft.relname::text,
                array(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord)
                      JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum ORDER BY k.ord),
                con.confdeltype::text, con.confupdtype::text, pg_get_constraintdef(con.oid)
         FROM pg_constraint con
         JOIN pg_class t ON t.oid = con.conrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         LEFT JOIN pg_class ft ON ft.oid = con.confrelid
         WHERE n.nspname = current_schema() AND t.relname::text <> ALL($1) AND con.contype IN ('p', 'u', 'f', 'c')
         ORDER BY t.relname, con.conname",
        &[&reserved],
    )? {
        let (table, name, kind): (String, String, String) = (r.get(0), r.get(1), r.get(2));
        let cols: Vec<String> = r.get(3);
        let (ftable, fcols): (Option<String>, Vec<String>) = (r.get(4), r.get(5));
        let (del, upd): (String, String) = (r.get(6), r.get(7));
        let def: String = r.get(8);
        let Some(columns) = tables.get_mut(&table) else { continue };
        match kind.as_str() {
            "p" => {
                for c in columns.iter_mut().filter(|c| cols.contains(&c.name)) {
                    c.primary_key = true;
                    c.nullable = false;
                }
            }
            "u" if cols.len() == 1 => {
                if let Some(c) = columns.iter_mut().find(|c| c.name == cols[0]) {
                    c.unique = true;
                }
            }
            "u" => notes.push(format!("constraint {name} on {table}: multi-column UNIQUE ({}) is not expressible in SDL", cols.join(", "))),
            "f" => match (ftable, cols.as_slice(), fcols.as_slice(), action(&del), action(&upd)) {
                (Some(ft), [col], [fcol], Some(on_delete), Some(on_update)) => {
                    if let Some(c) = columns.iter_mut().find(|c| &c.name == col) {
                        c.references = Some(ForeignKeyIR { table: ft, column: fcol.clone(), on_delete, on_update });
                    }
                }
                _ => notes.push(format!(
                    "foreign key {name} on {table}: multi-column or SET DEFAULT keys are not expressible in SDL"
                )),
            },
            _ => {
                // CHECK: keep the server's text; `drift::align` matches it to the expected expression
                let sql = def.strip_prefix("CHECK ").unwrap_or(&def).to_string();
                checks.entry(table).or_default().push(ConstraintIR { name, expr: ExprIR::Raw { sql } });
            }
        }
    }

    // ---- standalone indexes ----------------------------------------------
    let mut indexes: BTreeMap<String, Vec<IndexIR>> = BTreeMap::new();
    for r in client.query(
        "SELECT t.relname::text, i.relname::text, ix.indisunique,
                array(SELECT a.attname::text FROM unnest(ix.indkey::int2[]) WITH ORDINALITY k(attnum, ord)
                      JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.attnum
                      WHERE k.attnum > 0 ORDER BY k.ord),
                ix.indpred IS NOT NULL, ix.indexprs IS NOT NULL, am.amname::text
         FROM pg_index ix
         JOIN pg_class i ON i.oid = ix.indexrelid
         JOIN pg_class t ON t.oid = ix.indrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         JOIN pg_am am ON am.oid = i.relam
         WHERE n.nspname = current_schema() AND t.relkind IN ('r', 'p') AND t.relname::text <> ALL($1)
           AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = ix.indexrelid)
         ORDER BY t.relname, i.relname",
        &[&reserved],
    )? {
        let (table, name, unique): (String, String, bool) = (r.get(0), r.get(1), r.get(2));
        let cols: Vec<String> = r.get(3);
        let (partial, expression, method): (bool, bool, String) = (r.get(4), r.get(5), r.get(6));
        if unique || partial || expression || method != "btree" {
            notes.push(format!(
                "index {name} on {table} is unique, partial, an expression index or not btree; SDL cannot express it"
            ));
            continue;
        }
        indexes.entry(table).or_default().push(IndexIR { name, columns: cols });
    }

    // ---- standalone sequences (those owned by a column belong to it) -------
    let mut sequences: Vec<SequenceIR> = Vec::new();
    for r in client.query(
        "SELECT c.relname::text, s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache, s.seqcycle
         FROM pg_sequence s
         JOIN pg_class c ON c.oid = s.seqrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = current_schema()
           AND NOT EXISTS (
               SELECT 1 FROM pg_depend d
               WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid
                 AND d.refclassid = 'pg_class'::regclass AND d.deptype IN ('a', 'i'))
         ORDER BY c.relname",
        &[],
    )? {
        sequences.push(SequenceIR {
            name: r.get(0),
            start: r.get(1),
            increment: r.get(2),
            min: r.get(3),
            max: r.get(4),
            cache: r.get(5),
            cycle: r.get(6),
        });
    }
    sequences.sort_by(|a, b| a.name.cmp(&b.name));

    // ---- objects SDL has no place for: said so, never silently dropped -----------
    for r in client.query(
        "SELECT kind, name FROM (
             SELECT CASE c.relkind WHEN 'v' THEN 'view' ELSE 'materialized view' END AS kind, c.relname::text AS name
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = current_schema() AND c.relkind IN ('v', 'm')
           UNION ALL
             SELECT 'trigger', t.tgname::text || ' on ' || c.relname::text
             FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE NOT t.tgisinternal AND n.nspname = current_schema()
         ) o ORDER BY kind, name",
        &[],
    )? {
        let (kind, name): (String, String) = (r.get(0), r.get(1));
        notes.push(format!("{kind} {name} is not represented in the schema"));
    }

    // ---- assemble ---------------------------------------------------------
    let ir = SchemaIR {
        version: certo_sdl::IR_VERSION,
        tables: tables
            .into_iter()
            .map(|(name, columns)| TableIR {
                relationships: Vec::new(), // relationships have no database representation
                indexes: indexes.remove(&name).unwrap_or_default(),
                constraints: checks.remove(&name).unwrap_or_default(),
                columns,
                name,
            })
            .collect(),
        enums: enums.into_iter().map(|(name, variants)| EnumIR { name, variants }).collect(),
        types: composites.into_iter().map(|(name, fields)| CompositeIR { name, fields }).collect(),
        sequences,
    };
    Ok(LiveSchema { ir, notes })
}
