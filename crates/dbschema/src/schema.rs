use std::collections::HashMap;
use std::path::Path;
use serde::{Deserialize, Serialize};
use certo_ast::decl::{Decl, TypeBody};
use certo_ast::module::Module;
use certo_ast::types::TypeExpr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaColumn {
    pub name:     String,
    /// Canonical Certo type string (e.g. `"Int"`, `"Text"`, `"UUID"`).
    pub ty:       String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaTable {
    pub name:    String,
    pub columns: Vec<SchemaColumn>,
}

/// The schema representation — serialisable to/from `schema.json`.
///
/// Used in three ways:
///   1. Built from Certo `type` declarations (`Schema::from_module`)
///   2. Loaded from a committed snapshot (`Schema::load`)
///   3. Pulled from a live Postgres database (`pull::pull_schema`)
#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Schema {
    /// Certo schema version tag (bumped when the JSON format changes).
    #[serde(default = "schema_version")]
    pub version: u32,
    /// table name → table definition
    pub tables: HashMap<String, SchemaTable>,
}

fn schema_version() -> u32 { 1 }

impl Schema {
    /// Build from Certo `type` declarations in a parsed module.
    pub fn from_module(module: &Module) -> Self {
        let mut schema = Schema { version: 1, ..Default::default() };
        for sdecl in &module.decls {
            if let Decl::Type(t) = &sdecl.node {
                if let TypeBody::Record(rec) = &t.body {
                    let columns = rec.fields.iter().map(|f| SchemaColumn {
                        name:     f.name.node.clone(),
                        ty:       te_to_str(&f.ty.node),
                        nullable: f.optional,
                    }).collect();
                    schema.tables.insert(t.name.node.clone(), SchemaTable {
                        name: t.name.node.clone(),
                        columns,
                    });
                }
            }
        }
        schema
    }

    /// Load a committed schema snapshot from `schema.json`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let src = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        serde_json::from_str(&src)
            .map_err(|e| format!("cannot parse {}: {}", path.display(), e))
    }

    /// Save the schema as a `schema.json` snapshot.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("cannot serialise schema: {}", e))?;
        std::fs::write(path, json)
            .map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }

    pub fn has_table(&self, name: &str) -> bool {
        self.tables.contains_key(name)
    }

    pub fn column_type(&self, table: &str, column: &str) -> Option<&str> {
        self.tables.get(table)
            .and_then(|t| t.columns.iter().find(|c| c.name == column))
            .map(|c| c.ty.as_str())
    }

    /// Diff `self` (snapshot) against `live` (pulled from DB).
    /// Returns a list of human-readable drift messages.
    pub fn diff(&self, live: &Schema) -> Vec<DriftItem> {
        let mut items = Vec::new();

        // Tables in snapshot but missing from live DB.
        for name in self.tables.keys() {
            if !live.tables.contains_key(name) {
                items.push(DriftItem::TableMissing { table: name.clone() });
                continue;
            }
            // Column-level diff.
            let snap_cols: HashMap<&str, &SchemaColumn> =
                self.tables[name].columns.iter().map(|c| (c.name.as_str(), c)).collect();
            let live_cols: HashMap<&str, &SchemaColumn> =
                live.tables[name].columns.iter().map(|c| (c.name.as_str(), c)).collect();

            for (col_name, snap_col) in &snap_cols {
                match live_cols.get(col_name) {
                    None => items.push(DriftItem::ColumnMissing {
                        table:  name.clone(),
                        column: col_name.to_string(),
                    }),
                    Some(live_col) => {
                        if snap_col.ty != live_col.ty {
                            items.push(DriftItem::ColumnTypeDrift {
                                table:    name.clone(),
                                column:   col_name.to_string(),
                                snapshot: snap_col.ty.clone(),
                                live:     live_col.ty.clone(),
                            });
                        }
                        if snap_col.nullable != live_col.nullable {
                            items.push(DriftItem::NullabilityDrift {
                                table:    name.clone(),
                                column:   col_name.to_string(),
                                snapshot: snap_col.nullable,
                                live:     live_col.nullable,
                            });
                        }
                    }
                }
            }
            // Columns in live DB not in snapshot (informational).
            for col_name in live_cols.keys() {
                if !snap_cols.contains_key(col_name) {
                    items.push(DriftItem::ColumnExtra {
                        table:  name.clone(),
                        column: col_name.to_string(),
                    });
                }
            }
        }

        // Tables in live DB not in snapshot.
        for name in live.tables.keys() {
            if !self.tables.contains_key(name) {
                items.push(DriftItem::TableExtra { table: name.clone() });
            }
        }

        items.sort();
        items
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DriftItem {
    TableMissing    { table: String },
    TableExtra      { table: String },
    ColumnMissing   { table: String, column: String },
    ColumnExtra     { table: String, column: String },
    ColumnTypeDrift { table: String, column: String, snapshot: String, live: String },
    NullabilityDrift{ table: String, column: String, snapshot: bool,   live: bool   },
}

impl std::fmt::Display for DriftItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriftItem::TableMissing    { table }         => write!(f, "MISSING  table  {table}"),
            DriftItem::TableExtra      { table }         => write!(f, "EXTRA    table  {table}"),
            DriftItem::ColumnMissing   { table, column } => write!(f, "MISSING  column {table}.{column}"),
            DriftItem::ColumnExtra     { table, column } => write!(f, "EXTRA    column {table}.{column}"),
            DriftItem::ColumnTypeDrift { table, column, snapshot, live } =>
                write!(f, "TYPE     {table}.{column}  snapshot={snapshot}  live={live}"),
            DriftItem::NullabilityDrift { table, column, snapshot, live } =>
                write!(f, "NULLABLE {table}.{column}  snapshot={snapshot}  live={live}"),
        }
    }
}

/// Stringify a surface TypeExpr for comparison with migration column types.
pub fn te_to_str(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Named { path, args, .. } => {
            let name = path.segments.last()
                .map(|s| s.node.as_str()).unwrap_or("_");
            if args.is_empty() { name.to_string() }
            else {
                format!("{}<{}>", name, args.iter().map(|a| te_to_str(&a.node)).collect::<Vec<_>>().join(", "))
            }
        }
        TypeExpr::Option { inner, .. } => format!("{}?", te_to_str(&inner.node)),
        TypeExpr::Tuple { elements, .. } =>
            format!("({})", elements.iter().map(|e| te_to_str(&e.node)).collect::<Vec<_>>().join(", ")),
        TypeExpr::Fn { params, ret, .. } =>
            format!("({}) => {}", params.iter().map(|p| te_to_str(&p.node)).collect::<Vec<_>>().join(", "), te_to_str(&ret.node)),
        TypeExpr::Record { .. } => "{ .. }".into(),
        TypeExpr::Ptr { inner, .. } => format!("*{}", te_to_str(&inner.node)),
        TypeExpr::Param { name, .. } => name.node.clone(),
    }
}
