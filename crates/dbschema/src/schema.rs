use std::collections::HashMap;
use certo_ast::decl::{Decl, TypeBody};
use certo_ast::module::Module;
use certo_ast::types::TypeExpr;

/// A resolved column in the schema.
#[derive(Debug, Clone)]
pub struct SchemaColumn {
    pub name:     String,
    /// Canonical type string (e.g. `"Int"`, `"Text"`, `"UUID"`).
    pub ty:       String,
    pub nullable: bool,
}

/// A table (= record type declaration) known to the schema.
#[derive(Debug, Clone)]
pub struct SchemaTable {
    pub name:    String,
    pub columns: Vec<SchemaColumn>,
}

/// The in-memory schema derived from `type` declarations in the module.
#[derive(Default, Debug)]
pub struct Schema {
    /// table name → table definition
    pub tables: HashMap<String, SchemaTable>,
}

impl Schema {
    /// Build the schema from all `type` declarations that have record bodies.
    pub fn build(module: &Module) -> Self {
        let mut schema = Schema::default();
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

    pub fn has_table(&self, name: &str) -> bool {
        self.tables.contains_key(name)
    }

    pub fn column_type(&self, table: &str, column: &str) -> Option<&str> {
        self.tables.get(table)
            .and_then(|t| t.columns.iter().find(|c| c.name == column))
            .map(|c| c.ty.as_str())
    }
}

/// A column read live from `information_schema.columns` for schema-sync
/// (see `check_schema_sync`). `certo_type` is already mapped from the raw Postgres type
/// name (e.g. `"uuid"` → `"UUID"`) by the caller — this crate has no Postgres-specific
/// knowledge, it just compares two Certo-shaped type strings.
#[derive(Debug, Clone)]
pub struct LiveColumn {
    /// Postgres column name, e.g. `"customer_id"` (snake_case).
    pub name:       String,
    pub certo_type: String,
    pub nullable:   bool,
}

/// A table read live from the database for schema-sync.
#[derive(Debug, Clone)]
pub struct LiveTable {
    /// Postgres table name, e.g. `"orders"` (snake_case).
    pub name:    String,
    pub columns: Vec<LiveColumn>,
}

/// `orders` → `Orders` — same convention `certo db pull` uses to name generated types.
pub(crate) fn snake_to_pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

/// `customer_id` → `customerId` — same convention `certo db pull` uses to name generated fields.
pub(crate) fn snake_to_camel(s: &str) -> String {
    let mut parts = s.split('_');
    let first = parts.next().unwrap_or("").to_string();
    let rest: String = parts.map(|w| {
        let mut c = w.chars();
        match c.next() {
            None => String::new(),
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        }
    }).collect();
    first + &rest
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
