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
pub fn snake_to_pascal(s: &str) -> String {
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
pub fn snake_to_camel(s: &str) -> String {
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

/// Is one of `a`/`b` a bare `"Decimal"` and the other a parameterized
/// `"Decimal(p, s)"` (in either order, case-insensitively, ignoring a
/// trailing `?`)? Every place that compares a declared type against another
/// declared/introspected type — `check_migrations::types_compat`,
/// `check_schema_sync`, and `certo db diff` (`crates/cli`) — needs this same
/// carve-out or'd into its own existing comparison, mirroring
/// `certo_typeck::unify`'s rule that a bare `Ty::Decimal` unifies with any
/// `Ty::Decimal(Some(p, s))`. Without it, a project that hasn't opted into
/// declaring precision/scale would see every existing Decimal-typed column
/// reported as drift the moment the live database has real `NUMERIC(p,s)`
/// metadata, even though nothing about the code or database changed. Two
/// *different* parameterizations (`Decimal(10, 2)` vs `Decimal(19, 4)`)
/// still conflict — this only ever returns `true` when exactly one side is
/// bare.
pub fn decimal_bare_vs_param(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('?').to_lowercase();
    let b = b.trim_end_matches('?').to_lowercase();
    let is_bare  = |s: &str| s == "decimal";
    let is_param = |s: &str| s.starts_with("decimal(");
    (is_bare(&a) && is_param(&b)) || (is_param(&a) && is_bare(&b))
}

/// Same carve-out as `decimal_bare_vs_param`, for `Text` vs `BoundedText(n)`
/// (BACKLOG item 147) — mirrors `certo_typeck::unify`'s identical rule that
/// bare `Ty::Text` unifies freely with any `Ty::BoundedText`. Two different
/// `BoundedText` max lengths still conflict — only ever `true` when exactly
/// one side is bare `Text`.
pub fn bounded_text_bare_vs_param(a: &str, b: &str) -> bool {
    let a = a.trim_end_matches('?').to_lowercase();
    let b = b.trim_end_matches('?').to_lowercase();
    let is_bare  = |s: &str| s == "text";
    let is_param = |s: &str| s.starts_with("boundedtext(");
    (is_bare(&a) && is_param(&b)) || (is_param(&a) && is_bare(&b))
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
        TypeExpr::DecimalParam { precision, scale, .. } => format!("Decimal({}, {})", precision, scale),
        TypeExpr::BoundedTextParam { max_len, .. } => format!("BoundedText({})", max_len),
    }
}
