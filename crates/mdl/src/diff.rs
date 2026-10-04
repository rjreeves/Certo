//! SchemaIR -> SchemaIR diff.
//!
//! Ops are emitted in six phases so every op only references things that
//! already exist when it runs, and the result is deterministic (everything
//! iterates in name order):
//!
//!   1. drop changed/removed constraints, indexes, relationships
//!   2. create enums, add variants, create/alter composite types
//!   3. create tables (columns only)
//!   4. add/alter columns
//!   5. add relationships, indexes, constraints
//!   6. drop columns, tables, removed variants, types, enums, sequences
//!
//! Views bracket all of that: those that were removed or changed, or that read a table whose columns change, are dropped
//! before phase 1 and (when still wanted) created again after phase 6.
//!
//! Sequences are created in phase 2 (before any table, since a column
//! default may call `nextval`) and dropped last.

use crate::plan::{MigrationPlan, Op, PLAN_VERSION};
use certo_sdl::{ColumnIR, CompositeIR, SchemaIR, TableIR, TypeIR};
use std::collections::{BTreeMap, BTreeSet};

fn by_name<T>(items: &[T], name: impl Fn(&T) -> &str) -> BTreeMap<&str, &T> {
    items.iter().map(|i| (name(i), i)).collect()
}

/// Foreign keys travel as their own ops, so column comparison and snapshots
/// ignore them.
fn strip_fk(c: &ColumnIR) -> ColumnIR {
    ColumnIR { references: None, ..c.clone() }
}

/// Diff `before` -> `after`. `diff(x, x)` is always empty.
pub fn diff(before: &SchemaIR, after: &SchemaIR) -> MigrationPlan {
    let mut p = Phases::default();

    // ---- enums ------------------------------------------------------- //
    let be = by_name(&before.enums, |e| &e.name);
    let ae = by_name(&after.enums, |e| &e.name);
    for (name, e) in &ae {
        match be.get(name) {
            None => p.create.push(Op::CreateEnum { definition: (*e).clone() }),
            Some(old) => {
                for v in e.variants.iter().filter(|v| !old.variants.contains(v)) {
                    p.create.push(Op::AddEnumVariant { name: name.to_string(), variant: v.clone() });
                }
                for v in old.variants.iter().filter(|v| !e.variants.contains(v)) {
                    p.drop.push(Op::RemoveEnumVariant { name: name.to_string(), variant: v.clone() });
                }
            }
        }
    }
    for name in be.keys().filter(|n| !ae.contains_key(*n)) {
        p.drop_enums.push(Op::DropEnum { name: name.to_string() });
    }

    // ---- sequences ---------------------------------------------------- //
    // created before any table (a column default may call nextval), dropped last
    let bs = by_name(&before.sequences, |s| &s.name);
    let asq = by_name(&after.sequences, |s| &s.name);
    for (name, s) in &asq {
        match bs.get(name) {
            None => p.create.push(Op::CreateSequence { definition: (*s).clone() }),
            Some(old) if old != s => {
                p.create.push(Op::AlterSequence { before: (*old).clone(), after: (*s).clone() });
            }
            Some(_) => {}
        }
    }
    for name in bs.keys().filter(|n| !asq.contains_key(*n)) {
        p.drop_sequences.push(Op::DropSequence { name: name.to_string() });
    }

    // ---- composite types -------------------------------------------- //
    let bt = by_name(&before.types, |t| &t.name);
    let at = by_name(&after.types, |t| &t.name);
    let new_types: Vec<&CompositeIR> =
        at.iter().filter(|(n, _)| !bt.contains_key(*n)).map(|(_, t)| *t).collect();
    for t in topo(&new_types) {
        p.types.push(Op::CreateType { definition: t.clone() });
    }
    for (name, t) in &at {
        if let Some(old) = bt.get(name)
            && old != t
        {
            p.types.push(Op::AlterType { name: name.to_string(), before: (*old).clone(), after: (*t).clone() });
        }
    }
    let gone_types: Vec<&CompositeIR> =
        bt.iter().filter(|(n, _)| !at.contains_key(*n)).map(|(_, t)| *t).collect();
    // dependents first: reverse of creation order
    for t in topo(&gone_types).into_iter().rev() {
        p.drop_types.push(Op::DropType { name: t.name.clone() });
    }

    // ---- tables ------------------------------------------------------ //
    let btab = by_name(&before.tables, |t| &t.name);
    let atab = by_name(&after.tables, |t| &t.name);
    for (name, t) in &atab {
        match btab.get(name) {
            None => {
                // The definition already carries the columns, so diff against a
                // table that has them (minus foreign keys); only the extras
                // (foreign keys, relationships, indexes, constraints) become ops.
                let def = TableIR {
                    name: name.to_string(),
                    columns: t.columns.iter().map(strip_fk).collect(),
                    relationships: Vec::new(),
                    indexes: Vec::new(),
                    constraints: Vec::new(),
                    view: false,
                };
                p.tables.push(Op::CreateTable { definition: def.clone() });
                diff_members(&mut p, name, &def, t);
            }
            Some(old) => diff_members(&mut p, name, old, t),
        }
    }
    let gone: Vec<&TableIR> =
        btab.iter().filter(|(n, _)| !atab.contains_key(*n)).map(|(_, t)| *t).collect();
    for t in drop_order(&gone) {
        p.drop_tables.push(Op::DropTable { name: t.to_string() });
    }

    // ---- views ------------------------------------------------------- //
    let bv = by_name(&before.views, |v| &v.name);
    let av = by_name(&after.views, |v| &v.name);
    let changes_columns = |p: &Phases, table: &str| {
        p.columns
            .iter()
            .chain(&p.drop_columns)
            .chain(&p.drop_tables)
            .any(|o| matches!(o, Op::AlterColumn { table: t, .. } | Op::DropColumn { table: t, .. } | Op::DropTable { name: t } if t == table))
    };
    for (name, old) in &bv {
        let wanted = av.get(name);
        if wanted.is_none_or(|v| v != old) || changes_columns(&p, &old.from) {
            p.drop_views.push(Op::DropView { name: name.to_string() });
        }
    }
    for (name, v) in &av {
        match bv.get(name) {
            Some(old) if *old == *v && !changes_columns(&p, &v.from) => {}
            _ => p.create_views.push(Op::CreateView { definition: (*v).clone() }),
        }
    }

    p.finish()
}

/// Columns, relationships, indexes and constraints of one surviving/new table.
fn diff_members(p: &mut Phases, table: &str, old: &TableIR, new: &TableIR) {
    let t = table.to_string();

    let oc = by_name(&old.columns, |c| &c.name);
    let nc = by_name(&new.columns, |c| &c.name);
    for (name, c) in &nc {
        match oc.get(name) {
            None => p.columns.push(Op::AddColumn { table: t.clone(), column: strip_fk(c) }),
            Some(o) if strip_fk(o) != strip_fk(c) => p.columns.push(Op::AlterColumn {
                table: t.clone(),
                before: strip_fk(o),
                after: strip_fk(c),
            }),
            Some(_) => {}
        }
    }

    // foreign keys, keyed by the owning column
    let fks = |cols: &BTreeMap<&str, &ColumnIR>| -> BTreeMap<String, certo_sdl::ForeignKeyIR> {
        cols.iter()
            .filter_map(|(n, c)| c.references.clone().map(|r| (n.to_string(), r)))
            .collect()
    };
    let (ofk, nfk) = (fks(&oc), fks(&nc));
    for (col, r) in &nfk {
        if ofk.get(col) != Some(r) {
            if ofk.contains_key(col) {
                p.drop.push(Op::DropForeignKey { table: t.clone(), name: col.clone() });
            }
            p.add.push(Op::AddForeignKey { table: t.clone(), column: col.clone(), references: r.clone() });
        }
    }
    for col in ofk.keys().filter(|c| !nfk.contains_key(*c)) {
        p.drop.push(Op::DropForeignKey { table: t.clone(), name: col.clone() });
    }
    for name in oc.keys().filter(|n| !nc.contains_key(*n)) {
        p.drop_columns.push(Op::DropColumn { table: t.clone(), name: name.to_string() });
    }

    let or = by_name(&old.relationships, |r| &r.name);
    let nr = by_name(&new.relationships, |r| &r.name);
    for (name, r) in &nr {
        if or.get(name) != Some(r) {
            if or.contains_key(name) {
                p.drop.push(Op::DropRelationship { table: t.clone(), name: name.to_string() });
            }
            p.add.push(Op::AddRelationship { table: t.clone(), relationship: (*r).clone() });
        }
    }
    for name in or.keys().filter(|n| !nr.contains_key(*n)) {
        p.drop.push(Op::DropRelationship { table: t.clone(), name: name.to_string() });
    }

    let oi = by_name(&old.indexes, |i| &i.name);
    let ni = by_name(&new.indexes, |i| &i.name);
    for (name, i) in &ni {
        if oi.get(name) != Some(i) {
            if oi.contains_key(name) {
                p.drop.push(Op::DropIndex { table: t.clone(), name: name.to_string() });
            }
            p.add.push(Op::CreateIndex { table: t.clone(), index: (*i).clone() });
        }
    }
    for name in oi.keys().filter(|n| !ni.contains_key(*n)) {
        p.drop.push(Op::DropIndex { table: t.clone(), name: name.to_string() });
    }

    let ok = by_name(&old.constraints, |c| &c.name);
    let nk = by_name(&new.constraints, |c| &c.name);
    for (name, c) in &nk {
        if ok.get(name) != Some(c) {
            if ok.contains_key(name) {
                p.drop.push(Op::DropConstraint { table: t.clone(), name: name.to_string() });
            }
            p.add.push(Op::AddConstraint { table: t.clone(), constraint: (*c).clone() });
        }
    }
    for name in ok.keys().filter(|n| !nk.contains_key(*n)) {
        p.drop.push(Op::DropConstraint { table: t.clone(), name: name.to_string() });
    }
}

/// Tables (other than itself) that `t` has foreign keys to.
fn referenced(t: &TableIR) -> BTreeSet<&str> {
    t.columns
        .iter()
        .filter_map(|c| c.references.as_ref())
        .map(|r| r.table.as_str())
        .filter(|r| *r != t.name)
        .collect()
}

/// Order tables being dropped so a table is dropped before any table it
/// references. Foreign-key cycles fall back to name order (the adapter must
/// drop the constraints first).
fn drop_order<'a>(tables: &[&'a TableIR]) -> Vec<&'a str> {
    let mut remaining: BTreeSet<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    let deps: BTreeMap<&str, BTreeSet<&str>> =
        tables.iter().map(|t| (t.name.as_str(), referenced(t))).collect();
    let mut out = Vec::new();
    while !remaining.is_empty() {
        // a table is safe to drop once no remaining table references it
        let next = remaining
            .iter()
            .find(|n| !remaining.iter().any(|o| o != *n && deps[o].contains(*n)))
            .or_else(|| remaining.iter().next())
            .copied()
            .unwrap();
        remaining.remove(next);
        out.push(next);
    }
    out
}

/// Order composite types so each comes after the composites it contains.
fn topo<'a>(types: &[&'a CompositeIR]) -> Vec<&'a CompositeIR> {
    let names: BTreeSet<&str> = types.iter().map(|t| t.name.as_str()).collect();
    let map: BTreeMap<&str, &CompositeIR> = types.iter().map(|t| (t.name.as_str(), *t)).collect();
    let mut out = Vec::new();
    let mut done = BTreeSet::new();
    fn visit<'a>(
        n: &str,
        names: &BTreeSet<&str>,
        map: &BTreeMap<&str, &'a CompositeIR>,
        done: &mut BTreeSet<String>,
        out: &mut Vec<&'a CompositeIR>,
    ) {
        if !done.insert(n.to_string()) { return; }
        let t = map[n];
        for f in &t.fields {
            if let TypeIR::Composite(dep) = &f.ty
                && names.contains(dep.as_str())
            {
                visit(dep, names, map, done, out);
            }
        }
        out.push(t);
    }
    for n in &names {
        visit(n, &names, &map, &mut done, &mut out);
    }
    out
}

#[derive(Default)]
struct Phases {
    /// 1. drops that must precede changes (relationships, indexes, constraints, variants)
    drop: Vec<Op>,
    /// 2. creates
    create: Vec<Op>,
    types: Vec<Op>,
    tables: Vec<Op>,
    columns: Vec<Op>,
    add: Vec<Op>,
    /// 6. final drops
    drop_columns: Vec<Op>,
    drop_tables: Vec<Op>,
    drop_types: Vec<Op>,
    drop_enums: Vec<Op>,
    drop_sequences: Vec<Op>,
    /// views dropped before everything, created again after everything
    drop_views: Vec<Op>,
    create_views: Vec<Op>,
}

impl Phases {
    fn finish(self) -> MigrationPlan {
        // Variant removals belong with the final drops (after columns stop using them),
        // everything else in `drop` runs first.
        let (variants, early): (Vec<Op>, Vec<Op>) =
            self.drop.into_iter().partition(|o| matches!(o, Op::RemoveEnumVariant { .. }));
        let mut ops = Vec::new();
        ops.extend(self.drop_views);
        ops.extend(early);
        ops.extend(self.create);
        ops.extend(self.types);
        ops.extend(self.tables);
        ops.extend(self.columns);
        ops.extend(self.add);
        ops.extend(self.drop_columns);
        ops.extend(self.drop_tables);
        ops.extend(variants);
        ops.extend(self.drop_types);
        ops.extend(self.drop_enums);
        ops.extend(self.drop_sequences);
        ops.extend(self.create_views);
        MigrationPlan { version: PLAN_VERSION, ops }
    }
}
