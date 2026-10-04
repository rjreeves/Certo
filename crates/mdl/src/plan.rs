use certo_sdl::{
    ColumnIR, Generation, CompositeIR, ConstraintIR, EnumIR, ExprIR, ForeignKeyIR, IndexIR, ReferentialAction,
    RelationshipIR, SequenceIR, TableIR, TypeIR, ViewIR,
};
use serde::{Deserialize, Serialize};

pub const PLAN_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub version: u32,
    pub ops: Vec<Op>,
}

impl MigrationPlan {
    pub fn is_empty(&self) -> bool { self.ops.is_empty() }

    pub fn has_destructive(&self) -> bool { self.ops.iter().any(Op::is_destructive) }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("MigrationPlan is always serializable")
    }

    pub fn from_json(s: &str) -> Result<MigrationPlan, serde_json::Error> {
        serde_json::from_str(s)
    }
}

/// One schema change. `CreateTable` carries columns only (with `references`
/// cleared); the table's foreign keys, relationships, indexes and constraints
/// follow as separate ops so ops never reference something created later in
/// the plan. Likewise `AddColumn` / `AlterColumn` never carry a foreign key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    CreateEnum { definition: EnumIR },
    AddEnumVariant { name: String, variant: String },
    RemoveEnumVariant { name: String, variant: String },
    DropEnum { name: String },

    CreateSequence { definition: SequenceIR },
    AlterSequence { before: SequenceIR, after: SequenceIR },
    DropSequence { name: String },

    CreateType { definition: CompositeIR },
    AlterType { name: String, before: CompositeIR, after: CompositeIR },
    DropType { name: String },

    CreateTable { definition: TableIR },
    DropTable { name: String },

    /// Views come first (dropped) and last (created) in a plan: a view stops the table it reads from being changed.
    CreateView { definition: ViewIR },
    DropView { name: String },

    AddColumn { table: String, column: ColumnIR },
    AlterColumn { table: String, before: ColumnIR, after: ColumnIR },
    DropColumn { table: String, name: String },

    AddForeignKey { table: String, column: String, references: ForeignKeyIR },
    DropForeignKey { table: String, name: String },

    AddRelationship { table: String, relationship: RelationshipIR },
    DropRelationship { table: String, name: String },

    CreateIndex { table: String, index: IndexIR },
    DropIndex { table: String, name: String },

    AddConstraint { table: String, constraint: ConstraintIR },
    DropConstraint { table: String, name: String },

    // ---- produced by MDL directives (see `migration`) ----
    /// `columns` are the table's columns as they are *before* the rename, so
    /// an adapter can rename the constraints it derived from their names.
    RenameTable { from: String, to: String, columns: Vec<ColumnIR> },
    /// `table` is the table's name at this point (after any table rename);
    /// `column` is the column under its *new* name.
    RenameColumn { table: String, from: String, to: String, column: ColumnIR },
    /// Replace an enum type, moving rows of removed variants to a replacement.
    /// `columns` lists every column of the new schema that uses the enum.
    RecreateEnum {
        name: String,
        before: EnumIR,
        after: EnumIR,
        mappings: Vec<VariantMapping>,
        columns: Vec<EnumColumn>,
    },
    DataUpdate { table: String, set: Vec<Assignment>, filter: Option<ExprIR> },
    /// Fill NULLs in a column, then make it NOT NULL. Safe by construction,
    /// so it is never destructive.
    Backfill { table: String, column: String, value: ExprIR },
    /// Verbatim SQL; `dialect: None` applies to every dialect.
    RawSql { dialect: Option<String>, sql: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantMapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumColumn {
    pub table: String,
    pub column: ColumnIR,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub column: String,
    pub value: ExprIR,
}

impl Op {
    /// Can this op lose data or fail on existing rows?
    pub fn is_destructive(&self) -> bool {
        match self {
            Op::DropTable { .. }
            | Op::DropColumn { .. }
            | Op::DropEnum { .. }
            | Op::RemoveEnumVariant { .. }
            | Op::DropType { .. }
            | Op::DropSequence { .. }
            | Op::RecreateEnum { .. }
            | Op::DataUpdate { .. }
            | Op::RawSql { .. } => true,
            Op::AlterColumn { before, after, .. } => {
                (before.ty != after.ty && !widens(&before.ty, &after.ty))
                    || generation_lossy(before.generated, after.generated)
                    || (before.nullable && !after.nullable)
                    || (!before.unique && after.unique)
            }
            Op::AlterType { before, after, .. } => before
                .fields
                .iter()
                .any(|f| !after.fields.iter().any(|g| g.name == f.name && g.ty == f.ty)),
            _ => false,
        }
    }

    /// One-line human summary, prefixed `+` (add), `~` (change) or `-` (remove).
    pub fn describe(&self) -> String {
        match self {
            Op::CreateEnum { definition } => format!("+ enum {}", definition.name),
            Op::AddEnumVariant { name, variant } => format!("+ enum variant {name}.{variant}"),
            Op::RemoveEnumVariant { name, variant } => format!("- enum variant {name}.{variant}"),
            Op::DropEnum { name } => format!("- enum {name}"),
            Op::CreateSequence { definition } => format!("+ sequence {}", definition.name),
            Op::AlterSequence { after, .. } => format!("~ sequence {}", after.name),
            Op::DropSequence { name } => format!("- sequence {name}"),
            Op::CreateType { definition } => format!("+ type {}", definition.name),
            Op::AlterType { name, .. } => format!("~ type {name}"),
            Op::DropType { name } => format!("- type {name}"),
            Op::CreateTable { definition } => format!("+ table {}", definition.name),
            Op::DropTable { name } => format!("- table {name}"),
            Op::CreateView { definition } => format!("+ view {}", definition.name),
            Op::DropView { name } => format!("- view {name}"),
            Op::AddColumn { table, column } => format!("+ column {table}.{}", column.name),
            Op::AlterColumn { table, after, .. } => format!("~ column {table}.{}", after.name),
            Op::DropColumn { table, name } => format!("- column {table}.{name}"),
            Op::AddForeignKey { table, column, references } => {
                let mut s = format!("+ foreign key {table}.{column} -> {}.{}", references.table, references.column);
                for (kw, a) in [("delete", references.on_delete), ("update", references.on_update)] {
                    if a != ReferentialAction::NoAction {
                        s.push_str(&format!(" on {kw} {}", action_sql(a).to_lowercase()));
                    }
                }
                s
            }
            Op::DropForeignKey { table, name } => format!("- foreign key {table}.{name}"),
            Op::RenameTable { from, to, .. } => format!("~ rename table {from} -> {to}"),
            Op::RenameColumn { table, from, to, .. } => format!("~ rename column {table}.{from} -> {to}"),
            Op::RecreateEnum { name, mappings, .. } => {
                let m: Vec<_> = mappings.iter().map(|m| format!("{} -> {}", m.from, m.to)).collect();
                format!("~ recreate enum {name} (remap {})", m.join(", "))
            }
            Op::DataUpdate { table, .. } => format!("~ update data in {table}"),
            Op::Backfill { table, column, .. } => format!("~ backfill {table}.{column} (then NOT NULL)"),
            Op::RawSql { dialect, .. } => match dialect {
                Some(d) => format!("~ raw sql ({d})"),
                None => "~ raw sql".to_string(),
            },
            Op::AddRelationship { table, relationship } => format!("+ relationship {table}.{}", relationship.name),
            Op::DropRelationship { table, name } => format!("- relationship {table}.{name}"),
            Op::CreateIndex { table, index } => format!("+ index {} on {table}", index.name),
            Op::DropIndex { table, name } => format!("- index {name} on {table}"),
            Op::AddConstraint { table, constraint } => format!("+ constraint {} on {table}", constraint.name),
            Op::DropConstraint { table, name } => format!("- constraint {name} on {table}"),
        }
    }
}

/// SQL keyword form of an action (shared by summaries and SQL lowering).
pub fn action_sql(a: ReferentialAction) -> &'static str {
    match a {
        ReferentialAction::NoAction => "NO ACTION",
        ReferentialAction::Restrict => "RESTRICT",
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
    }
}

/// Is `from` -> `to` a change that keeps every existing value (a widening)?
/// Anything not listed is treated as potentially lossy.
pub fn widens(from: &TypeIR, to: &TypeIR) -> bool {
    use certo_sdl::Builtin::*;
    let (TypeIR::Builtin(f), TypeIR::Builtin(t)) = (from, to) else { return false };
    match (f, t) {
        (SmallInt, Int | BigInt | Decimal) | (Int, BigInt | Decimal) | (BigInt, Decimal) | (Real, Float) => true,
        (Varchar(a), Varchar(b)) | (Char(a), Char(b)) | (Char(a), Varchar(b)) => b >= a,
        (Varchar(_) | Char(_), Text) => true,
        (Numeric(..), Decimal) => true,
        // more integer digits AND at least as much scale
        (Numeric(p1, s1), Numeric(p2, s2)) => s2 >= s1 && p2.saturating_sub(*s2) >= p1.saturating_sub(*s1),
        _ => false,
    }
}

/// Does changing a column's generation throw away a counter? Removing it
/// does, and so does swapping serial for identity (a new sequence is built).
/// Adding a generation, or flipping identity between ALWAYS and BY DEFAULT, keeps everything.
pub fn generation_lossy(from: Option<Generation>, to: Option<Generation>) -> bool {
    use Generation::*;
    match (from, to) {
        (a, b) if a == b => false,
        (Some(_), None) => true,
        (Some(Serial), Some(_)) | (Some(_), Some(Serial)) => true,
        _ => false,
    }
}
