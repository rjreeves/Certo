//! SchemaIR: the canonical, engine-agnostic, serializable schema.
//!
//! Deterministic by construction: tables, enums and types are sorted by
//! name; columns keep declaration order; indexes and constraints are sorted
//! by name within their table. Parentheses and source spans are gone, so two
//! textually different but equivalent schemas produce identical JSON.

use crate::ast::{BinaryOp, Cardinality, Generation, ReferentialAction};
use serde::{Deserialize, Serialize};

pub const IR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaIR {
    pub version: u32,
    pub tables: Vec<TableIR>,
    pub enums: Vec<EnumIR>,
    pub types: Vec<CompositeIR>,
    #[serde(default)]
    pub sequences: Vec<SequenceIR>,
    /// Views declared in SDL (`view name on table (...) where ...`), sorted by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<ViewIR>,
}

impl SchemaIR {
    /// A schema with nothing in it; the "before" side of an initial migration.
    pub fn empty() -> SchemaIR {
        SchemaIR { version: IR_VERSION, tables: Vec::new(), enums: Vec::new(), types: Vec::new(), sequences: Vec::new(), views: Vec::new() }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("SchemaIR is always serializable")
    }

    pub fn from_json(s: &str) -> Result<SchemaIR, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn table(&self, name: &str) -> Option<&TableIR> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn view(&self, name: &str) -> Option<&ViewIR> {
        self.views.iter().find(|v| v.name == name)
    }

    /// The schema as the query language sees it: every SDL view added as a read-only table.
    pub fn with_views_as_tables(&self) -> SchemaIR {
        let mut out = self.clone();
        for v in &self.views {
            if let Some(t) = v.as_table(self) {
                out.tables.push(t);
            }
        }
        out.tables.sort_by(|a, b| a.name.cmp(&b.name));
        out.views.clear();
        out
    }
}

/// A view declared in SDL: some of one table's columns, and optionally only the rows a condition keeps.
/// Deliberately small: no joins, no computed columns, no other views, so every database means the same thing by it
/// and a diff can tell exactly what changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewIR {
    pub name: String,
    /// The table it reads.
    pub from: String,
    /// Which of its columns, in the order shown.
    pub columns: Vec<String>,
    /// Rows kept; `None` keeps all.
    #[serde(default)]
    pub filter: Option<ExprIR>,
}

impl ViewIR {
    /// What a reader of the view sees: the chosen columns with their types and nullability, and nothing that only
    /// makes sense on a table (keys, defaults, generation). `None` if the table or a column is missing.
    pub fn as_table(&self, schema: &SchemaIR) -> Option<TableIR> {
        let base = schema.table(&self.from)?;
        let columns = self
            .columns
            .iter()
            .map(|c| {
                base.column(c).map(|c| ColumnIR {
                    name: c.name.clone(),
                    ty: c.ty.clone(),
                    primary_key: false,
                    unique: false,
                    nullable: c.nullable && !c.primary_key,
                    default: None,
                    references: None,
                    generated: None,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(TableIR {
            name: self.name.clone(),
            columns,
            relationships: Vec::new(),
            indexes: Vec::new(),
            constraints: Vec::new(),
            view: true,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableIR {
    pub name: String,
    pub columns: Vec<ColumnIR>,
    pub relationships: Vec<RelationshipIR>,
    pub indexes: Vec<IndexIR>,
    pub constraints: Vec<ConstraintIR>,
    /// A persisted view (a QL `view`), not a table: it can be read like one but never written, and it is never part of a
    /// schema diff. Only the query language's own copy of the schema has these.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub view: bool,
}

impl TableIR {
    pub fn column(&self, name: &str) -> Option<&ColumnIR> {
        self.columns.iter().find(|c| c.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnIR {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeIR,
    pub primary_key: bool,
    pub unique: bool,
    pub nullable: bool,
    pub default: Option<ExprIR>,
    /// Foreign key, fully resolved to the referenced table and column.
    #[serde(default)]
    pub references: Option<ForeignKeyIR>,
    /// Auto-generated value (`serial` / identity). Implies NOT NULL and no default.
    #[serde(default)]
    pub generated: Option<Generation>,
}

/// A standalone sequence, fully resolved (no defaults left implicit) so it
/// compares exactly with what a database reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceIR {
    pub name: String,
    pub start: i64,
    pub increment: i64,
    pub min: i64,
    pub max: i64,
    pub cache: i64,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ForeignKeyIR {
    pub table: String,
    pub column: String,
    #[serde(default)]
    pub on_delete: ReferentialAction,
    #[serde(default)]
    pub on_update: ReferentialAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationshipIR {
    pub name: String,
    pub target: String,
    pub cardinality: Cardinality,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexIR {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintIR {
    pub name: String,
    pub expr: ExprIR,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumIR {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompositeIR {
    pub name: String,
    pub fields: Vec<FieldIR>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldIR {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: TypeIR,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum TypeIR {
    Builtin(Builtin),
    Enum(String),
    Composite(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Builtin {
    Text,
    SmallInt,
    Int,
    BigInt,
    /// Unbounded `numeric`.
    Decimal,
    Real,
    Float,
    Bool,
    Uuid,
    /// Timestamp WITH time zone (the everyday choice).
    Timestamp,
    /// Timestamp WITHOUT time zone.
    #[serde(rename = "timestamp_naive")]
    TimestampNaive,
    Date,
    Json,
    Bytes,
    /// `varchar(n)`
    Varchar(u32),
    /// `char(n)`
    Char(u32),
    /// `decimal(precision, scale)`
    Numeric(u16, u16),
}

impl Builtin {
    /// Types written as a bare name in SDL.
    pub fn from_name(s: &str) -> Option<Builtin> {
        Some(match s {
            "text" => Builtin::Text,
            "smallint" => Builtin::SmallInt,
            "int" => Builtin::Int,
            "bigint" => Builtin::BigInt,
            "decimal" => Builtin::Decimal,
            "real" => Builtin::Real,
            "float" => Builtin::Float,
            "bool" => Builtin::Bool,
            "uuid" => Builtin::Uuid,
            "timestamp" => Builtin::Timestamp,
            "timestamp_naive" => Builtin::TimestampNaive,
            "date" => Builtin::Date,
            "json" => Builtin::Json,
            "bytes" => Builtin::Bytes,
            _ => return None,
        })
    }

    /// Names that are types (bare or parameterised); declarations may not reuse them.
    pub fn is_type_name(s: &str) -> bool {
        Builtin::from_name(s).is_some()
            || matches!(s, "varchar" | "char" | "numeric" | "serial" | "bigserial" | "smallserial")
    }

    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Builtin::SmallInt | Builtin::Int | Builtin::BigInt | Builtin::Decimal | Builtin::Real | Builtin::Float | Builtin::Numeric(..)
        )
    }

    /// How SDL writes this type.
    pub fn sdl_name(self) -> String {
        match self {
            Builtin::Text => "text".into(),
            Builtin::SmallInt => "smallint".into(),
            Builtin::Int => "int".into(),
            Builtin::BigInt => "bigint".into(),
            Builtin::Decimal => "decimal".into(),
            Builtin::Real => "real".into(),
            Builtin::Float => "float".into(),
            Builtin::Bool => "bool".into(),
            Builtin::Uuid => "uuid".into(),
            Builtin::Timestamp => "timestamp".into(),
            Builtin::TimestampNaive => "timestamp_naive".into(),
            Builtin::Date => "date".into(),
            Builtin::Json => "json".into(),
            Builtin::Bytes => "bytes".into(),
            Builtin::Varchar(n) => format!("varchar({n})"),
            Builtin::Char(n) => format!("char({n})"),
            Builtin::Numeric(p, 0) => format!("decimal({p})"),
            Builtin::Numeric(p, s) => format!("decimal({p},{s})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExprIR {
    Number { value: i64 },
    /// Decimal text as written, sign included (`-1.5`); never rounded.
    Decimal { value: String },
    String { value: String },
    Bool { value: bool },
    Column { name: String },
    EnumVariant { enum_name: String, variant: String },
    Call { func: String, args: Vec<ExprIR> },
    Binary { op: BinaryOp, lhs: Box<ExprIR>, rhs: Box<ExprIR> },
    /// `nextval(sequence)`
    NextVal { sequence: String },
    Not { expr: Box<ExprIR> },
    IsNull { expr: Box<ExprIR>, negated: bool },
    In { expr: Box<ExprIR>, list: Vec<ExprIR>, negated: bool },
    /// SQL text read back from a live database that SDL cannot parse (a
    /// default or CHECK body as the server normalised it). Never produced by
    /// compiling SDL; only by introspection. Equal only to identical text.
    Raw { sql: String },
}
