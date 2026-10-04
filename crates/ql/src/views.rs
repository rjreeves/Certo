//! Persisted views: `view name { from ... select ... }`, written in QL and kept in the database as a view.
//!
//! ```text
//! view paid_orders { from orders o where o.paid select o.id, o.customer_id, o.total }
//! view big_spenders { from customers c join paid_orders p on p.customer_id == c.id group by c.id, c.name
//!                     select c.id, c.name, sum(p.total) as spent }
//! query top() { from big_spenders b order by b.spent desc limit 10 select b.name, b.spent }
//! ```
//!
//! A view is checked like a query with no parameters and lowered to `CREATE VIEW`. Its result columns are its typed
//! contract, and the rest of the file (and anything compiled against the schema this returns) reads it as a table:
//! a `TableIR` flagged `view`, never part of a schema diff, never writable (QL263). A view may read tables, fragments and
//! other views, in any order (they are created in dependency order; a cycle is QL262). Which views exist in a database, and
//! when they are dropped and recreated around migrations, is the runner's job, not this module's.

use crate::ast::*;
use crate::check::check;
use crate::ir::*;
use crate::lower::{lower, Lowered};
use certo_diagnostics::{Diagnostic, Severity};
use certo_sdl::{ColumnIR, SchemaIR, TableIR};
use certo_sql::{quote_ident, Dialect};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// A checked view: its name, typed columns, the table the query language sees, and the SQL to create and drop it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ViewOut {
    pub name: String,
    pub columns: Vec<ColumnOut>,
    /// What the rest of the language reads: the view as a (read-only) table.
    pub table: TableIR,
    pub create_sql: String,
    pub drop_sql: String,
}

/// Check the views of `file` and return them in the order they must be created, with a copy of `schema` that also has them
/// as tables (earlier views are visible to later ones, and to every statement). Errors go to `diags`.
pub fn resolve(schema: &SchemaIR, file: &QlFile, dialect: Dialect, diags: &mut Vec<Diagnostic>) -> (SchemaIR, Vec<ViewOut>) {
    let mut aug = schema.clone();
    if file.views.is_empty() {
        return (aug, Vec::new());
    }

    // ---- names ---------------------------------------------------------------------------------------------------
    let mut by_name: BTreeMap<&str, &ViewDecl> = BTreeMap::new();
    for v in &file.views {
        if by_name.insert(v.name.name.as_str(), v).is_some() {
            diags.push(Diagnostic::error("QL262", format!("view `{}` is defined more than once", v.name.name)).with_span(v.name.span));
        }
        if schema.tables.iter().any(|t| t.name == v.name.name) {
            diags.push(
                Diagnostic::error("QL262", format!("view `{}` has the same name as a table: pick another name", v.name.name)).with_span(v.name.span),
            );
        }
        if file.fragments.iter().any(|f| f.name.name == v.name.name) {
            diags.push(
                Diagnostic::error("QL262", format!("view `{}` has the same name as a fragment: pick another name", v.name.name)).with_span(v.name.span),
            );
        }
    }
    if has_errors(diags) {
        return (aug, Vec::new());
    }

    // ---- dependency order: a view is created after the views it reads ------------------------------------------------
    let names: HashSet<&str> = by_name.keys().copied().collect();
    let deps: BTreeMap<&str, BTreeSet<String>> = by_name
        .iter()
        .map(|(n, v)| {
            let mut used = BTreeSet::new();
            crate::fragments::tables_in_query(&v.query, &mut used);
            used.retain(|t| names.contains(t.as_str()));
            (*n, used)
        })
        .collect();
    let mut order: Vec<&str> = Vec::new();
    let mut state: BTreeMap<&str, u8> = BTreeMap::new(); // 1 = on the path, 2 = done
    fn visit<'a>(
        n: &'a str,
        deps: &BTreeMap<&'a str, BTreeSet<String>>,
        names: &HashSet<&'a str>,
        state: &mut BTreeMap<&'a str, u8>,
        order: &mut Vec<&'a str>,
        path: &mut Vec<&'a str>,
    ) -> Result<(), Vec<String>> {
        match state.get(n) {
            Some(2) => return Ok(()),
            Some(1) => {
                let at = path.iter().position(|p| *p == n).unwrap_or(0);
                let mut cycle: Vec<String> = path[at..].iter().map(|s| s.to_string()).collect();
                cycle.push(n.to_string());
                return Err(cycle);
            }
            _ => {}
        }
        state.insert(n, 1);
        path.push(n);
        for d in &deps[n] {
            let d: &'a str = names.get(d.as_str()).copied().expect("a known view");
            visit(d, deps, names, state, order, path)?;
        }
        path.pop();
        state.insert(n, 2);
        order.push(n);
        Ok(())
    }
    for n in by_name.keys().copied() {
        if let Err(cycle) = visit(n, &deps, &names, &mut state, &mut order, &mut Vec::new()) {
            diags.push(
                Diagnostic::error("QL262", format!("views cannot read themselves, directly or through others: {}", cycle.join(" -> ")))
                    .with_span(by_name[cycle[0].as_str()].name.span),
            );
            return (aug, Vec::new());
        }
    }

    // ---- each view, checked against the schema plus the views before it ----------------------------------------------
    let mut out = Vec::new();
    for n in order {
        let v = by_name[n];
        let alone = QlFile { queries: vec![v.query.clone()], mutations: Vec::new(), fragments: Vec::new(), views: Vec::new() };
        let mut local = Vec::new();
        let checked = check(&aug, &alone, &mut local);
        let failed = has_errors(&local);
        diags.append(&mut local);
        let (Some(ir), false) = (checked.into_iter().next(), failed) else { continue };
        if !ir.params.is_empty() {
            continue; // reported as an unknown parameter already
        }
        let Lowered { sql, .. } = lower(dialect, &ir);
        let q = quote_ident(dialect, &v.name.name);
        let columns: Vec<ColumnIR> = ir
            .select
            .iter()
            .map(|c| ColumnIR {
                name: c.name.clone(),
                ty: c.ty.clone(),
                primary_key: false,
                unique: false,
                nullable: c.nullable,
                default: None,
                references: None,
                generated: None,
            })
            .collect();
        let table = TableIR { name: v.name.name.clone(), columns, relationships: vec![], indexes: vec![], constraints: vec![], view: true };
        aug.tables.push(table.clone());
        out.push(ViewOut {
            name: v.name.name.clone(),
            columns: ir.select.clone(),
            table,
            create_sql: format!("CREATE VIEW {q} AS\n{sql}"),
            drop_sql: format!("DROP VIEW IF EXISTS {q}"),
        });
    }
    (aug, out)
}

fn has_errors(d: &[Diagnostic]) -> bool { d.iter().any(|x| x.severity == Severity::Error) }
