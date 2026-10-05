//! Apply an MDL file to (old schema, new schema): validate every directive
//! against the schemas, then produce a plan that honours them.
//!
//! Plan layout: `before` blocks, renames, the generated diff (rewritten for
//! `remap` and `backfill`), `after` blocks.
//!
//! Renames are applied to a copy of the old schema *before* diffing, so the
//! diff sees only real changes; the rename ops themselves come first in the
//! plan, and every later op uses the post-rename names.
//!
//! Error codes: MDL301 unknown in old schema/table, MDL302 missing from the
//! new schema, MDL303 conflicting name, MDL304 duplicate directive, MDL305
//! enum used by a composite type, MDL306 removed variant without remap,
//! MDL310-316 data steps (table, column, type, condition, duplicate
//! assignment, dialect, empty SQL), MDL320 directive with no effect.

use crate::diff::diff;
use crate::lang::{self, Directive, MdlFile, Step, When};
use crate::plan::{Assignment, EnumColumn, MigrationPlan, Op, VariantMapping};
use certo_ast::span::Span;
use certo_diagnostics::{Diagnostic, Severity};
use certo_sdl::{
    default_assignable, describe_type, ColumnIR, ExprCx, ExprIR, Ident, SchemaIR, TableIR, TypeIR,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// SQL dialect names accepted by `sql <dialect> "..."`.
pub const KNOWN_DIALECTS: &[&str] = &["postgres", "sqlite", "mysql"];

/// Parse and apply an MDL source. The plan is `Some` only if there are no
/// error diagnostics; diagnostics point into `mdl_src`.
pub fn compile_migration(
    before: &SchemaIR,
    after: &SchemaIR,
    mdl_src: &str,
) -> (Option<MigrationPlan>, Vec<Diagnostic>) {
    let (file, mut diags) = lang::parse(mdl_src);
    if has_errors(&diags) {
        return (None, diags);
    }
    let plan = plan_migration(before, after, &file, &mut diags);
    (plan, diags)
}

fn has_errors(d: &[Diagnostic]) -> bool {
    d.iter().any(|x| x.severity == Severity::Error)
}

struct Cx<'a> {
    diags: &'a mut Vec<Diagnostic>,
    failed: bool,
}

impl Cx<'_> {
    fn fail(&mut self, code: &str, msg: impl Into<String>, span: Span) {
        self.failed = true;
        self.diags.push(Diagnostic::error(code, msg).with_span(span));
    }
}

fn enum_variants(ir: &SchemaIR) -> HashMap<String, Vec<String>> {
    ir.enums.iter().map(|e| (e.name.clone(), e.variants.clone())).collect()
}

fn sequence_names(ir: &SchemaIR) -> HashSet<String> {
    ir.sequences.iter().map(|s| s.name.clone()).collect()
}

fn column_types(t: &TableIR) -> HashMap<String, TypeIR> {
    t.columns.iter().map(|c| (c.name.clone(), c.ty.clone())).collect()
}

struct Backfill {
    table: String,
    column: String,
    value: ExprIR,
    span: Span,
}

pub fn plan_migration(
    before: &SchemaIR,
    after: &SchemaIR,
    file: &MdlFile,
    diags: &mut Vec<Diagnostic>,
) -> Option<MigrationPlan> {
    let mut cx = Cx { diags, failed: false };

    let (table_renames, col_renames) = collect_renames(before, after, file, &mut cx);
    let remaps = collect_remaps(before, after, file, &mut cx);
    let backfills = collect_backfills(after, file, &mut cx);
    let (before_ops, after_ops) = collect_blocks(before, after, file, &mut cx);
    if cx.failed {
        return None;
    }

    let renamed = apply_renames(before, &table_renames, &col_renames);
    let mut plan = diff(&renamed, after);
    apply_remaps(&mut plan, &renamed, after, &remaps);
    apply_backfills(&mut plan, &backfills, &mut cx);
    if cx.failed {
        return None;
    }

    // ---- assemble ---------------------------------------------------- //
    // views that must go come before everything, the renames included
    let lead = plan.ops.iter().take_while(|o| matches!(o, Op::DropView { .. })).count();
    let rest = plan.ops.split_off(lead);
    let mut ops = std::mem::replace(&mut plan.ops, rest);
    ops.extend(before_ops);
    for (from, to) in &table_renames {
        let columns = before.tables.iter().find(|t| &t.name == from).map(|t| t.columns.clone()).unwrap_or_default();
        ops.push(Op::RenameTable { from: from.clone(), to: to.clone(), columns });
    }
    for (table, from, to) in &col_renames {
        let new_table = table_renames.iter().find(|(f, _)| f == table).map_or(table, |(_, t)| t);
        let column = before
            .tables
            .iter()
            .find(|t| &t.name == table)
            .and_then(|t| t.columns.iter().find(|c| &c.name == from))
            .map(|c| ColumnIR { name: to.clone(), ..c.clone() });
        if let Some(column) = column {
            ops.push(Op::RenameColumn { table: new_table.clone(), from: from.clone(), to: to.clone(), column });
        }
    }
    ops.extend(plan.ops);
    ops.extend(after_ops);
    plan.ops = ops;
    recreate_views_over_renames(&mut plan, &renamed, after, &table_renames, &col_renames);
    Some(plan)
}

/// A rename, or an enum rebuilt under it, changes what a view reads without changing the view: it has to be dropped
/// before and created after like any other view over a table being altered.
fn recreate_views_over_renames(
    plan: &mut MigrationPlan,
    renamed: &SchemaIR,
    after: &SchemaIR,
    table_renames: &TableRenames,
    col_renames: &ColRenames,
) {
    let mut touched: HashSet<String> = table_renames.iter().map(|(_, to)| to.clone()).collect();
    for (table, _, _) in col_renames {
        touched.insert(table_renames.iter().find(|(f, _)| f == table).map_or(table, |(_, t)| t).clone());
    }
    for op in &plan.ops {
        if let Op::RecreateEnum { columns, .. } = op {
            touched.extend(columns.iter().map(|c| c.table.clone()));
        }
    }
    for v in &after.views {
        if !touched.contains(&v.from) { continue; }
        let drops = plan.ops.iter().any(|o| matches!(o, Op::DropView { name } if *name == v.name));
        let creates = plan.ops.iter().any(|o| matches!(o, Op::CreateView { definition } if definition.name == v.name));
        if !drops && renamed.view(&v.name).is_some() {
            plan.ops.insert(0, Op::DropView { name: v.name.clone() });
        }
        if !creates {
            plan.ops.push(Op::CreateView { definition: v.clone() });
        }
    }
}

// ---- renames --------------------------------------------------------- //

type TableRenames = Vec<(String, String)>;
type ColRenames = Vec<(String, String, String)>;

fn collect_renames(before: &SchemaIR, after: &SchemaIR, file: &MdlFile, cx: &mut Cx) -> (TableRenames, ColRenames) {
    let mut tables: TableRenames = Vec::new();
    let mut cols: ColRenames = Vec::new();

    for d in &file.directives {
        let Directive::RenameTable { from, to, span } = d else { continue };
        if tables.iter().any(|(f, t)| *f == from.name || *t == to.name) {
            cx.fail("MDL304", format!("table `{}` or `{}` is already part of another rename", from.name, to.name), *span);
            continue;
        }
        if before.table(&from.name).is_none() {
            cx.fail("MDL301", format!("unknown table `{}` in the old schema", from.name), from.span);
        } else if after.table(&to.name).is_none() {
            cx.fail("MDL302", format!("table `{}` does not exist in the new schema", to.name), to.span);
        } else if before.table(&to.name).is_some() {
            cx.fail("MDL303", format!("`{}` already exists in the old schema", to.name), to.span);
        } else if after.table(&from.name).is_some() {
            cx.fail("MDL303", format!("`{}` still exists in the new schema; a rename must remove the old name", from.name), from.span);
        } else {
            tables.push((from.name.clone(), to.name.clone()));
        }
    }

    for d in &file.directives {
        let Directive::RenameColumn { table, from, to, span } = d else { continue };
        if cols.iter().any(|(t, f, n)| *t == table.name && (*f == from.name || *n == to.name)) {
            cx.fail("MDL304", format!("column `{}.{}` or `{}` is already part of another rename", table.name, from.name, to.name), *span);
            continue;
        }
        let Some(old_t) = before.table(&table.name) else {
            cx.fail("MDL301", format!("unknown table `{}` in the old schema", table.name), table.span);
            continue;
        };
        let new_name = tables.iter().find(|(f, _)| *f == table.name).map_or(&table.name, |(_, t)| t);
        let Some(new_t) = after.table(new_name) else {
            cx.fail("MDL302", format!("table `{new_name}` does not exist in the new schema"), table.span);
            continue;
        };
        if old_t.column(&from.name).is_none() {
            cx.fail("MDL301", format!("unknown column `{}.{}` in the old schema", table.name, from.name), from.span);
        } else if new_t.column(&to.name).is_none() {
            cx.fail("MDL302", format!("column `{new_name}.{}` does not exist in the new schema", to.name), to.span);
        } else if old_t.column(&to.name).is_some() {
            cx.fail("MDL303", format!("`{}.{}` already exists in the old schema", table.name, to.name), to.span);
        } else if new_t.column(&from.name).is_some() {
            cx.fail("MDL303", format!("`{new_name}.{}` still exists in the new schema; a rename must remove the old name", from.name), from.span);
        } else {
            cols.push((table.name.clone(), from.name.clone(), to.name.clone()));
        }
    }
    (tables, cols)
}

fn rename_in_expr(e: &mut ExprIR, from: &str, to: &str) {
    match e {
        ExprIR::Column { name } if name == from => *name = to.to_string(),
        ExprIR::Call { args, .. } => args.iter_mut().for_each(|a| rename_in_expr(a, from, to)),
        ExprIR::Binary { lhs, rhs, .. } => {
            rename_in_expr(lhs, from, to);
            rename_in_expr(rhs, from, to);
        }
        ExprIR::Not { expr } | ExprIR::IsNull { expr, .. } => rename_in_expr(expr, from, to),
        ExprIR::In { expr, list, .. } => {
            rename_in_expr(expr, from, to);
            list.iter_mut().for_each(|e| rename_in_expr(e, from, to));
        }
        ExprIR::Number { .. }
        | ExprIR::Decimal { .. }
        | ExprIR::String { .. }
        | ExprIR::Bool { .. }
        | ExprIR::Column { .. }
        | ExprIR::EnumVariant { .. }
        | ExprIR::NextVal { .. }
        | ExprIR::Raw { .. } => {}
    }
}

/// The old schema with the renames already applied, including everything
/// that refers to a renamed name (keys, indexes, constraints, relationships).
fn apply_renames(ir: &SchemaIR, tables: &TableRenames, cols: &ColRenames) -> SchemaIR {
    let mut out = ir.clone();
    for (t, from, to) in cols {
        for table in &mut out.tables {
            if table.name == *t {
                for c in &mut table.columns {
                    if c.name == *from { c.name = to.clone(); }
                }
                for i in &mut table.indexes {
                    for c in &mut i.columns {
                        if c == from { *c = to.clone(); }
                    }
                }
                for k in &mut table.constraints {
                    rename_in_expr(&mut k.expr, from, to);
                }
            }
            for c in &mut table.columns {
                if let Some(r) = &mut c.references
                    && r.table == *t
                    && r.column == *from
                {
                    r.column = to.clone();
                }
            }
        }
    }
    for (from, to) in tables {
        for table in &mut out.tables {
            if table.name == *from { table.name = to.clone(); }
            for c in &mut table.columns {
                if let Some(r) = &mut c.references
                    && r.table == *from
                {
                    r.table = to.clone();
                }
            }
            for rel in &mut table.relationships {
                if rel.target == *from { rel.target = to.clone(); }
            }
        }
    }
    for (t, from, to) in cols {
        for v in out.views.iter_mut().filter(|v| v.from == *t) {
            for c in &mut v.columns {
                if c == from { *c = to.clone(); }
            }
            if let Some(f) = &mut v.filter {
                rename_in_expr(f, from, to);
            }
        }
    }
    for (from, to) in tables {
        for v in &mut out.views {
            if v.from == *from { v.from = to.clone(); }
        }
    }
    out.tables.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---- remaps ---------------------------------------------------------- //

/// enum -> [(removed variant, replacement)]
fn collect_remaps(before: &SchemaIR, after: &SchemaIR, file: &MdlFile, cx: &mut Cx) -> BTreeMap<String, Vec<VariantMapping>> {
    let mut out: BTreeMap<String, Vec<VariantMapping>> = BTreeMap::new();
    let mut first_span: HashMap<String, Span> = HashMap::new();

    for d in &file.directives {
        let Directive::Remap { enum_name, from, to, span } = d else { continue };
        let old = before.enums.iter().find(|e| e.name == enum_name.name);
        let new = after.enums.iter().find(|e| e.name == enum_name.name);
        let (Some(old), Some(new)) = (old, new) else {
            cx.fail("MDL301", format!("enum `{}` must exist in both the old and the new schema", enum_name.name), enum_name.span);
            continue;
        };
        if out.get(&enum_name.name).is_some_and(|m| m.iter().any(|x| x.from == from.name)) {
            cx.fail("MDL304", format!("variant `{}` is remapped twice", from.name), *span);
        } else if !old.variants.contains(&from.name) {
            cx.fail("MDL301", format!("`{}` is not a variant of `{}` in the old schema", from.name, enum_name.name), from.span);
        } else if new.variants.contains(&from.name) {
            cx.fail("MDL303", format!("`{}` is still a variant of `{}` in the new schema; only removed variants are remapped", from.name, enum_name.name), from.span);
        } else if !new.variants.contains(&to.name) {
            cx.fail("MDL302", format!("`{}` is not a variant of `{}` in the new schema", to.name, enum_name.name), to.span);
        } else {
            first_span.entry(enum_name.name.clone()).or_insert(*span);
            out.entry(enum_name.name.clone()).or_default().push(VariantMapping { from: from.name.clone(), to: to.name.clone() });
        }
    }

    for (name, maps) in &out {
        let old = before.enums.iter().find(|e| &e.name == name).unwrap();
        let new = after.enums.iter().find(|e| &e.name == name).unwrap();
        let span = first_span[name];
        for v in old.variants.iter().filter(|v| !new.variants.contains(v)) {
            if !maps.iter().any(|m| &m.from == v) {
                cx.fail("MDL306", format!("variant `{v}` of `{name}` is removed but has no `remap {name}.{v} -> ...`"), span);
            }
        }
        for t in before.types.iter().chain(&after.types) {
            if t.fields.iter().any(|f| f.ty == TypeIR::Enum(name.clone())) {
                cx.fail("MDL305", format!("enum `{name}` is used by composite type `{}`; recreate it by hand with `sql`", t.name), span);
                break;
            }
        }
    }
    out
}

/// Replace each remapped enum's `RemoveEnumVariant` ops with one `RecreateEnum`.
fn apply_remaps(plan: &mut MigrationPlan, renamed: &SchemaIR, after: &SchemaIR, remaps: &BTreeMap<String, Vec<VariantMapping>>) {
    if remaps.is_empty() {
        return;
    }
    let mut placed: HashSet<String> = HashSet::new();
    let mut ops = Vec::new();
    for op in plan.ops.drain(..) {
        match &op {
            Op::RemoveEnumVariant { name, .. } if remaps.contains_key(name) => {
                if placed.insert(name.clone()) {
                    let before_enum = renamed.enums.iter().find(|e| &e.name == name).cloned().unwrap();
                    let after_enum = after.enums.iter().find(|e| &e.name == name).cloned().unwrap();
                    let columns = after
                        .tables
                        .iter()
                        .flat_map(|t| {
                            t.columns
                                .iter()
                                .filter(|c| c.ty == TypeIR::Enum(name.clone()))
                                .map(|c| EnumColumn { table: t.name.clone(), column: c.clone() })
                        })
                        .collect();
                    ops.push(Op::RecreateEnum {
                        name: name.clone(),
                        before: before_enum,
                        after: after_enum,
                        mappings: remaps[name].clone(),
                        columns,
                    });
                }
            }
            _ => ops.push(op),
        }
    }
    plan.ops = ops;
}

// ---- backfills ------------------------------------------------------- //

fn collect_backfills(after: &SchemaIR, file: &MdlFile, cx: &mut Cx) -> Vec<Backfill> {
    let enums = enum_variants(after);
    let seqs = sequence_names(after);
    let mut out: Vec<Backfill> = Vec::new();
    for d in &file.directives {
        let Directive::Backfill { table, column, value, span } = d else { continue };
        if out.iter().any(|b| b.table == table.name && b.column == column.name) {
            cx.fail("MDL304", format!("`{}.{}` is backfilled twice", table.name, column.name), *span);
            continue;
        }
        let Some(t) = after.table(&table.name) else {
            cx.fail("MDL302", format!("table `{}` does not exist in the new schema", table.name), table.span);
            continue;
        };
        let Some(col) = t.column(&column.name) else {
            cx.fail("MDL311", format!("table `{}` has no column `{}` in the new schema", table.name, column.name), column.span);
            continue;
        };
        let cols = column_types(t);
        if let Some(v) = check_assignment(cx, &enums, &seqs, &cols, &col.ty, &column.name, value) {
            out.push(Backfill { table: table.name.clone(), column: column.name.clone(), value: v, span: *span });
        }
    }
    out
}

/// Add the column as nullable; `Backfill` then fills it and makes it NOT NULL.
fn apply_backfills(plan: &mut MigrationPlan, backfills: &[Backfill], cx: &mut Cx) {
    for b in backfills {
        let idx = plan.ops.iter().position(|op| match op {
            Op::AddColumn { table, column } => *table == b.table && column.name == b.column && !column.nullable,
            Op::AlterColumn { table, before, after } => {
                *table == b.table && after.name == b.column && before.nullable && !after.nullable
            }
            _ => false,
        });
        let Some(idx) = idx else {
            cx.fail(
                "MDL320",
                format!("backfill of `{}.{}` has no effect: this migration neither adds it as NOT NULL nor makes it NOT NULL", b.table, b.column),
                b.span,
            );
            continue;
        };
        match &mut plan.ops[idx] {
            Op::AddColumn { column, .. } => column.nullable = true,
            Op::AlterColumn { after, .. } => after.nullable = true,
            _ => unreachable!(),
        }
        // fill (and tighten) once every column change has been made
        let pos = plan
            .ops
            .iter()
            .rposition(|op| matches!(op, Op::AddColumn { .. } | Op::AlterColumn { .. }))
            .map_or(idx + 1, |p| p + 1);
        plan.ops.insert(pos, Op::Backfill { table: b.table.clone(), column: b.column.clone(), value: b.value.clone() });
    }
}

// ---- before / after blocks ------------------------------------------ //

fn collect_blocks(before: &SchemaIR, after: &SchemaIR, file: &MdlFile, cx: &mut Cx) -> (Vec<Op>, Vec<Op>) {
    let (mut early, mut late) = (Vec::new(), Vec::new());
    for d in &file.directives {
        let Directive::Block { when, steps, .. } = d else { continue };
        let (schema, out) = match when {
            When::Before => (before, &mut early),
            When::After => (after, &mut late),
        };
        let enums = enum_variants(schema);
        let seqs = sequence_names(schema);
        for s in steps {
            match s {
                Step::Sql { dialect, sql, span } => {
                    if sql.trim().is_empty() {
                        cx.fail("MDL316", "SQL step is empty", *span);
                    } else if let Some(d) = dialect
                        && !KNOWN_DIALECTS.contains(&d.name.as_str())
                    {
                        cx.fail("MDL315", format!("unknown SQL dialect `{}` (supported: {})", d.name, KNOWN_DIALECTS.join(", ")), d.span);
                    } else {
                        out.push(Op::RawSql { dialect: dialect.as_ref().map(|d| d.name.clone()), sql: sql.clone() });
                    }
                }
                Step::Update { table, set, filter, .. } => {
                    if let Some(op) = check_update(cx, schema, &enums, &seqs, table, set, filter.as_ref()) {
                        out.push(op);
                    }
                }
            }
        }
    }
    (early, late)
}

fn check_update(
    cx: &mut Cx,
    schema: &SchemaIR,
    enums: &HashMap<String, Vec<String>>,
    seqs: &HashSet<String>,
    table: &Ident,
    set: &[lang::Assign],
    filter: Option<&certo_sdl::Expr>,
) -> Option<Op> {
    let Some(t) = schema.table(&table.name) else {
        cx.fail("MDL310", format!("unknown table `{}`", table.name), table.span);
        return None;
    };
    let cols = column_types(t);
    let mut assignments: Vec<Assignment> = Vec::new();
    let mut ok = true;
    for a in set {
        let Some(ty) = cols.get(&a.column.name) else {
            cx.fail("MDL311", format!("table `{}` has no column `{}`", table.name, a.column.name), a.column.span);
            ok = false;
            continue;
        };
        if assignments.iter().any(|x| x.column == a.column.name) {
            cx.fail("MDL314", format!("column `{}` is assigned twice", a.column.name), a.column.span);
            ok = false;
            continue;
        }
        match check_assignment(cx, enums, seqs, &cols, ty, &a.column.name, &a.value) {
            Some(v) => assignments.push(Assignment { column: a.column.name.clone(), value: v }),
            None => ok = false,
        }
    }
    let mut cond = None;
    if let Some(f) = filter {
        let mut ecx = ExprCx { enums, columns: Some(&cols), sequences: seqs, diags: cx.diags };
        match ecx.check(f, None) {
            Some((e, TypeIR::Builtin(certo_sdl::Builtin::Bool))) => cond = Some(e),
            Some((_, ty)) => {
                cx.fail("MDL313", format!("`where` must be a boolean expression, found {}", describe_type(&ty)), f.span());
                ok = false;
            }
            None => {
                cx.failed = true;
                ok = false;
            }
        }
    }
    ok.then(|| Op::DataUpdate { table: table.name.clone(), set: assignments, filter: cond })
}

/// Type-check `value` against a column's type; reports and returns `None` on failure.
fn check_assignment(
    cx: &mut Cx,
    enums: &HashMap<String, Vec<String>>,
    seqs: &HashSet<String>,
    cols: &HashMap<String, TypeIR>,
    want: &TypeIR,
    column: &str,
    value: &certo_sdl::Expr,
) -> Option<ExprIR> {
    let mut ecx = ExprCx { enums, columns: Some(cols), sequences: seqs, diags: cx.diags };
    let Some((ir, got)) = ecx.check(value, Some(want)) else {
        cx.failed = true;
        return None;
    };
    if !default_assignable(want, &got, value) {
        cx.fail(
            "MDL312",
            format!("value has type {}, but column `{column}` is {}", describe_type(&got), describe_type(want)),
            value.span(),
        );
        return None;
    }
    Some(ir)
}
