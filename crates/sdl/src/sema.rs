//! Semantic analysis and SchemaIR construction.
//!
//! Order matters: enums, then composite types (with cycle detection), then
//! tables (resolving column types and defaults), then indexes and
//! constraints, which need every table's column types.

use crate::ast::*;
use crate::expr::{default_assignable, describe, ExprCx};
use crate::ir::*;
use crate::symbols::{SymKind, SymbolTable};
use certo_ast::span::Span;
use certo_diagnostics::Diagnostic;
use std::collections::{BTreeMap, HashMap, HashSet};

fn error(diags: &mut Vec<Diagnostic>, code: &str, msg: String, span: Span) {
    diags.push(Diagnostic::error(code, msg).with_span(span));
}

/// Analyse a parsed file and build its SchemaIR. Errors go to `diags`; the
/// returned IR is only meaningful when no error diagnostics were added.
pub fn analyze(file: &SdlFile, diags: &mut Vec<Diagnostic>) -> SchemaIR {
    let sym = SymbolTable::collect(file, diags);

    // ---- enums ------------------------------------------------------- //
    let mut enums: BTreeMap<String, EnumIR> = BTreeMap::new();
    let mut variants: HashMap<String, Vec<String>> = HashMap::new();
    for d in &file.decls {
        let Decl::Enum(e) = d else { continue };
        if !sym.is_canonical(&e.name) { continue; }
        let mut seen = HashSet::new();
        let mut list = Vec::new();
        for v in &e.variants {
            if seen.insert(v.name.clone()) {
                list.push(v.name.clone());
            } else {
                error(diags, "SDL204", format!("duplicate variant `{}` in enum `{}`", v.name, e.name.name), v.span);
            }
        }
        variants.insert(e.name.name.clone(), list.clone());
        enums.insert(e.name.name.clone(), EnumIR { name: e.name.name.clone(), variants: list });
    }

    // ---- sequences --------------------------------------------------- //
    let mut sequences: BTreeMap<String, SequenceIR> = BTreeMap::new();
    for d in &file.decls {
        let Decl::Sequence(s) = d else { continue };
        if !sym.is_canonical(&s.name) { continue; }
        if let Some(ir) = resolve_sequence(s, diags) {
            sequences.insert(ir.name.clone(), ir);
        }
    }
    let seq_names: HashSet<String> = sequences.keys().cloned().collect();

    // ---- composite types -------------------------------------------- //
    let mut composites: BTreeMap<String, CompositeIR> = BTreeMap::new();
    let mut composite_spans: HashMap<String, Span> = HashMap::new();
    for d in &file.decls {
        let Decl::Type(t) = d else { continue };
        if !sym.is_canonical(&t.name) { continue; }
        let mut seen = HashSet::new();
        let mut fields = Vec::new();
        for f in &t.fields {
            if !seen.insert(f.name.name.clone()) {
                error(diags, "SDL205", format!("duplicate field `{}` in type `{}`", f.name.name, t.name.name), f.name.span);
                continue;
            }
            if let Some(ty) = resolve_type(&sym, &f.ty, diags) {
                fields.push(FieldIR { name: f.name.name.clone(), ty });
            }
        }
        composite_spans.insert(t.name.name.clone(), t.name.span);
        composites.insert(t.name.name.clone(), CompositeIR { name: t.name.name.clone(), fields });
    }
    for (name, span) in &composite_spans {
        if reaches(name, name, &composites, &mut HashSet::new()) {
            error(diags, "SDL212", format!("composite type `{name}` contains itself"), *span);
        }
    }

    // ---- tables ------------------------------------------------------ //
    let mut tables: BTreeMap<String, TableIR> = BTreeMap::new();
    let mut col_types: HashMap<String, HashMap<String, TypeIR>> = HashMap::new();
    for d in &file.decls {
        let Decl::Table(t) = d else { continue };
        if !sym.is_canonical(&t.name) { continue; }
        let (ir, types) = build_table(t, &sym, &variants, &seq_names, diags);
        col_types.insert(ir.name.clone(), types);
        tables.insert(ir.name.clone(), ir);
    }

    resolve_references(file, &sym, &mut tables, diags);

    // ---- indexes and constraints ------------------------------------ //
    for d in &file.decls {
        match d {
            Decl::Index(i) if sym.is_canonical_index(&i.name) => {
                let Some(cols) = table_cols(&col_types, &i.table, diags) else { continue };
                let mut seen = HashSet::new();
                let mut names = Vec::new();
                for c in &i.columns {
                    if !cols.contains_key(&c.name) {
                        error(diags, "SDL206", format!("table `{}` has no column `{}`", i.table.name, c.name), c.span);
                    } else if !seen.insert(c.name.clone()) {
                        error(diags, "SDL206", format!("column `{}` listed twice in index `{}`", c.name, i.name.name), c.span);
                    } else {
                        names.push(c.name.clone());
                    }
                }
                if names.len() == i.columns.len() {
                    tables.get_mut(&i.table.name).unwrap().indexes
                        .push(IndexIR { name: i.name.name.clone(), columns: names });
                }
            }
            Decl::Constraint(c) if sym.is_canonical_constraint(&c.name) => {
                let Some(cols) = table_cols(&col_types, &c.table, diags) else { continue };
                let mut cx = ExprCx { enums: &variants, columns: Some(cols), sequences: &seq_names, diags };
                let Some((expr, ty)) = cx.check(&c.expr, None) else { continue };
                if ty != TypeIR::Builtin(Builtin::Bool) {
                    error(diags, "SDL211",
                        format!("constraint `{}` must be a boolean expression, found {}", c.name.name, describe(&ty)),
                        c.expr.span());
                    continue;
                }
                tables.get_mut(&c.table.name).unwrap().constraints
                    .push(ConstraintIR { name: c.name.name.clone(), expr });
            }
            _ => {}
        }
    }
    for t in tables.values_mut() {
        t.indexes.sort_by(|a, b| a.name.cmp(&b.name));
        t.constraints.sort_by(|a, b| a.name.cmp(&b.name));
    }

    SchemaIR {
        version: IR_VERSION,
        tables: tables.into_values().collect(),
        enums: enums.into_values().collect(),
        types: composites.into_values().collect(),
        sequences: sequences.into_values().collect(),
    }
}

/// Fill in PostgreSQL's defaults so the IR never carries an implicit value.
fn resolve_sequence(s: &SequenceDecl, diags: &mut Vec<Diagnostic>) -> Option<SequenceIR> {
    let mut get: HashMap<SeqOptKind, i64> = HashMap::new();
    let mut cycle = false;
    for o in &s.options {
        let dup = match (o.kind, o.value) {
            (SeqOptKind::Cycle, _) => std::mem::replace(&mut cycle, true),
            (k, Some(v)) => get.insert(k, v).is_some(),
            _ => false,
        };
        if dup {
            error(diags, "SDL253", format!("duplicate option in sequence `{}`", s.name.name), o.span);
            return None;
        }
    }
    let increment = get.get(&SeqOptKind::Increment).copied().unwrap_or(1);
    if increment == 0 {
        error(diags, "SDL254", format!("sequence `{}`: increment cannot be 0", s.name.name), s.span);
        return None;
    }
    let (dmin, dmax) = if increment > 0 { (1, i64::MAX) } else { (i64::MIN, -1) };
    let min = get.get(&SeqOptKind::Min).copied().unwrap_or(dmin);
    let max = get.get(&SeqOptKind::Max).copied().unwrap_or(dmax);
    let start = get.get(&SeqOptKind::Start).copied().unwrap_or(if increment > 0 { min } else { max });
    let cache = get.get(&SeqOptKind::Cache).copied().unwrap_or(1);
    let problem = if min > max {
        Some("min must not exceed max")
    } else if start < min || start > max {
        Some("start must lie between min and max")
    } else if cache < 1 {
        Some("cache must be at least 1")
    } else {
        None
    };
    if let Some(p) = problem {
        error(diags, "SDL254", format!("sequence `{}`: {p}", s.name.name), s.span);
        return None;
    }
    Some(SequenceIR { name: s.name.name.clone(), start, increment, min, max, cache, cycle })
}

fn table_cols<'a>(
    col_types: &'a HashMap<String, HashMap<String, TypeIR>>,
    table: &Ident,
    diags: &mut Vec<Diagnostic>,
) -> Option<&'a HashMap<String, TypeIR>> {
    let r = col_types.get(&table.name);
    if r.is_none() {
        error(diags, "SDL206", format!("unknown table `{}`", table.name), table.span);
    }
    r
}

/// `varchar(n)`, `char(n)`, `decimal(p[, s])` and the errors for misused parameters.
fn resolve_parameterised(ty: &TypeRef, diags: &mut Vec<Diagnostic>) -> Option<TypeIR> {
    let name = ty.name.name.as_str();
    let mut fail = |code: &str, msg: String| {
        diags.push(Diagnostic::error(code, msg).with_span(ty.span));
        None
    };
    let b = |x: Builtin| Some(TypeIR::Builtin(x));
    match (name, ty.args.as_slice()) {
        ("varchar" | "char", [n]) if (1..=10_485_760).contains(n) => {
            b(if name == "varchar" { Builtin::Varchar(*n) } else { Builtin::Char(*n) })
        }
        ("varchar" | "char", [_]) => fail("SDL241", format!("`{name}` length must be between 1 and 10485760")),
        ("varchar" | "char", _) => fail("SDL241", format!("`{name}` needs exactly one length, e.g. `{name}(255)`")),
        ("decimal" | "numeric", []) => b(Builtin::Decimal),
        ("decimal" | "numeric", [p]) if (1..=1000).contains(p) => b(Builtin::Numeric(*p as u16, 0)),
        ("decimal" | "numeric", [p, s]) if (1..=1000).contains(p) && s <= p => b(Builtin::Numeric(*p as u16, *s as u16)),
        ("decimal" | "numeric", _) => {
            fail("SDL241", "`decimal` takes (precision) or (precision, scale), with precision 1-1000 and scale <= precision".into())
        }
        (other, _) => fail("SDL240", format!("type `{other}` does not take parameters")),
    }
}

fn resolve_type(sym: &SymbolTable, ty: &TypeRef, diags: &mut Vec<Diagnostic>) -> Option<TypeIR> {
    if serial_alias(ty).is_some() {
        diags.push(
            Diagnostic::error("SDL252", format!("`{}` is only available for table columns", ty.name.name))
                .with_span(ty.span)
                .with_note("use smallint, int or bigint in a composite type"),
        );
        return None;
    }
    if !ty.args.is_empty() || matches!(ty.name.name.as_str(), "varchar" | "char" | "numeric") {
        return resolve_parameterised(ty, diags);
    }
    let id = &ty.name;
    if let Some(b) = Builtin::from_name(&id.name) {
        return Some(TypeIR::Builtin(b));
    }
    match sym.kind(&id.name) {
        Some(SymKind::Enum) => Some(TypeIR::Enum(id.name.clone())),
        Some(SymKind::Type) => Some(TypeIR::Composite(id.name.clone())),
        Some(SymKind::Sequence) => {
            error(diags, "SDL202", format!("`{}` is a sequence, not a type", id.name), id.span);
            None
        }
        Some(SymKind::Table) => {
            diags.push(
                Diagnostic::error("SDL202", format!("`{}` is a table, not a type", id.name))
                    .with_span(id.span)
                    .with_note("use `name: Table -> one|optional|many` to declare a relationship"),
            );
            None
        }
        None => {
            error(diags, "SDL202", format!("unknown type `{}`", id.name), id.span);
            None
        }
    }
}

/// Does composite `cur` (transitively) contain composite `target`?
fn reaches(target: &str, cur: &str, all: &BTreeMap<String, CompositeIR>, seen: &mut HashSet<String>) -> bool {
    let Some(c) = all.get(cur) else { return false };
    for f in &c.fields {
        if let TypeIR::Composite(n) = &f.ty {
            if n == target { return true; }
            if seen.insert(n.clone()) && reaches(target, n, all, seen) { return true; }
        }
    }
    false
}

fn build_table(
    t: &TableDecl,
    sym: &SymbolTable,
    variants: &HashMap<String, Vec<String>>,
    seqs: &HashSet<String>,
    diags: &mut Vec<Diagnostic>,
) -> (TableIR, HashMap<String, TypeIR>) {
    let mut columns = Vec::new();
    let mut rels = Vec::new();
    let mut types = HashMap::new();
    let mut seen = HashSet::new();

    for m in &t.members {
        let (name, span) = match m {
            Member::Column(c) => (&c.name, c.span),
            Member::Rel(r) => (&r.name, r.span),
        };
        if !seen.insert(name.name.clone()) {
            error(diags, "SDL201", format!("duplicate member `{}` in table `{}`", name.name, t.name.name), span);
            continue;
        }
        match m {
            Member::Column(c) => {
                if let Some(col) = build_column(c, sym, variants, seqs, diags) {
                    types.insert(col.name.clone(), col.ty.clone());
                    columns.push(col);
                }
            }
            Member::Rel(r) => {
                if sym.kind(&r.target.name) != Some(SymKind::Table) {
                    error(diags, "SDL203", format!("relationship target `{}` is not a table", r.target.name), r.target.span);
                } else {
                    rels.push(RelationshipIR {
                        name: r.name.name.clone(),
                        target: r.target.name.clone(),
                        cardinality: r.cardinality,
                    });
                }
            }
        }
    }

    if !columns.iter().any(|c| c.primary_key) {
        diags.push(
            Diagnostic::warning("SDL220", format!("table `{}` has no primary key", t.name.name))
                .with_span(t.name.span),
        );
    }
    let ir = TableIR {
        name: t.name.name.clone(),
        columns,
        relationships: rels,
        indexes: Vec::new(),
        constraints: Vec::new(),
    };
    (ir, types)
}

/// `serial` / `bigserial` / `smallserial`: an integer column plus `Generation::Serial`.
fn serial_alias(ty: &TypeRef) -> Option<(Builtin, Generation)> {
    if !ty.args.is_empty() { return None; }
    Some(match ty.name.name.as_str() {
        "serial" => (Builtin::Int, Generation::Serial),
        "bigserial" => (Builtin::BigInt, Generation::Serial),
        "smallserial" => (Builtin::SmallInt, Generation::Serial),
        _ => return None,
    })
}

fn build_column(
    c: &ColumnDecl,
    sym: &SymbolTable,
    variants: &HashMap<String, Vec<String>>,
    seqs: &HashSet<String>,
    diags: &mut Vec<Diagnostic>,
) -> Option<ColumnIR> {
    let (ty, mut generated) = match serial_alias(&c.ty) {
        Some((b, g)) => (TypeIR::Builtin(b), Some(g)),
        None => (resolve_type(sym, &c.ty, diags)?, None),
    };
    let (mut pk, mut unique, mut not_null, mut null, mut refs) = (false, false, false, false, false);
    let mut default: Option<&Expr> = None;
    for m in &c.mods {
        let (flag, span, label) = match m {
            ColumnMod::PrimaryKey(s) => (&mut pk, *s, "primary key"),
            ColumnMod::Unique(s) => (&mut unique, *s, "unique"),
            ColumnMod::NotNull(s) => (&mut not_null, *s, "not null"),
            ColumnMod::Null(s) => (&mut null, *s, "null"),
            ColumnMod::References { span, .. } => (&mut refs, *span, "references"),
            ColumnMod::Generated { kind, span } => {
                if generated.is_some() {
                    error(diags, "SDL250", format!("column `{}` is already auto-generated", c.name.name), *span);
                } else {
                    generated = Some(*kind);
                }
                continue;
            }
            ColumnMod::Default(e) => {
                if default.is_some() {
                    error(diags, "SDL207", format!("column `{}` has more than one default", c.name.name), e.span());
                }
                default.get_or_insert(e);
                continue;
            }
        };
        if *flag {
            error(diags, "SDL207", format!("duplicate `{label}` on column `{}`", c.name.name), span);
        }
        *flag = true;
    }
    if null && (not_null || pk || generated.is_some()) {
        let what = if pk { "a primary key" } else if generated.is_some() { "auto-generated" } else { "`not null`" };
        error(diags, "SDL207", format!("column `{}` cannot be both nullable and {what}", c.name.name), c.span);
    }
    if generated.is_some() {
        if !matches!(ty, TypeIR::Builtin(Builtin::SmallInt | Builtin::Int | Builtin::BigInt)) {
            error(diags, "SDL251", format!("auto-generated column `{}` must be smallint, int or bigint", c.name.name), c.span);
        }
        if let Some(d) = default {
            error(diags, "SDL250", format!("auto-generated column `{}` cannot also have a default", c.name.name), d.span());
            default = None;
        }
    }

    let default = default.and_then(|e| {
        if matches!(ty, TypeIR::Composite(_)) {
            error(diags, "SDL208", "composite columns cannot have defaults".into(), e.span());
            return None;
        }
        let mut cx = ExprCx { enums: variants, columns: None, sequences: seqs, diags };
        let (ir, got) = cx.check(e, Some(&ty))?;
        if !default_assignable(&ty, &got, e) {
            error(diags, "SDL208",
                format!("default has type {}, but column `{}` is {}", describe(&got), c.name.name, describe(&ty)),
                e.span());
            return None;
        }
        Some(ir)
    });

    Some(ColumnIR {
        name: c.name.name.clone(),
        ty,
        primary_key: pk,
        unique,
        nullable: !(pk || not_null || generated.is_some()),
        default,
        references: None, // resolved once every table exists (see `resolve_references`)
        generated,
    })
}

/// Resolve `references` modifiers now that every table's columns are known.
/// The target column must be the target's sole primary key or a unique
/// column (defaulting to the sole primary key) and must have the same type.
fn resolve_references(
    file: &SdlFile,
    sym: &SymbolTable,
    tables: &mut BTreeMap<String, TableIR>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut resolved: Vec<(String, String, ForeignKeyIR)> = Vec::new();
    for d in &file.decls {
        let Decl::Table(t) = d else { continue };
        if !sym.is_canonical(&t.name) { continue; }
        for m in &t.members {
            let Member::Column(c) = m else { continue };
            for md in &c.mods {
                let ColumnMod::References { table, column, on_delete, on_update, span } = md else { continue };
                let Some(col) = tables.get(&t.name.name).and_then(|tb| tb.column(&c.name.name)) else { continue };
                let Some(target) = tables.get(&table.name) else {
                    error(diags, "SDL230", format!("unknown table `{}` in `references`", table.name), table.span);
                    continue;
                };
                let pk_count = target.columns.iter().filter(|x| x.primary_key).count();
                let key = match column {
                    Some(cn) => match target.column(&cn.name) {
                        Some(tc) if tc.unique || (tc.primary_key && pk_count == 1) => tc,
                        Some(_) => {
                            error(diags, "SDL231",
                                format!("`{}.{}` is not unique, so it cannot be referenced", table.name, cn.name), cn.span);
                            continue;
                        }
                        None => {
                            error(diags, "SDL231", format!("table `{}` has no column `{}`", table.name, cn.name), cn.span);
                            continue;
                        }
                    },
                    None => match target.columns.iter().find(|x| x.primary_key) {
                        Some(pk) if pk_count == 1 => pk,
                        _ => {
                            error(diags, "SDL231",
                                format!("table `{0}` has no single primary key; write `references {0}(column)`", table.name),
                                *span);
                            continue;
                        }
                    },
                };
                if key.ty != col.ty {
                    error(diags, "SDL232",
                        format!("column `{}` is {} but references `{}.{}` which is {}",
                            c.name.name, describe(&col.ty), table.name, key.name, describe(&key.ty)),
                        *span);
                    continue;
                }
                let (on_delete, on_update) = (on_delete.unwrap_or_default(), on_update.unwrap_or_default());
                if !col.nullable
                    && (on_delete == ReferentialAction::SetNull || on_update == ReferentialAction::SetNull)
                {
                    error(diags, "SDL233",
                        format!("`set null` requires column `{}` to be nullable", c.name.name), *span);
                    continue;
                }
                resolved.push((
                    t.name.name.clone(),
                    c.name.name.clone(),
                    ForeignKeyIR { table: table.name.clone(), column: key.name.clone(), on_delete, on_update },
                ));
            }
        }
    }
    for (table, column, fk) in resolved {
        if let Some(col) = tables.get_mut(&table).and_then(|t| t.columns.iter_mut().find(|c| c.name == column)) {
            col.references = Some(fk);
        }
    }
}
