use certo_ast::decl::*;
use crate::printer::ind;
use crate::fmt_type::{fmt_type, fmt_type_params, fmt_effects};
use crate::fmt_expr::{fmt_expr, fmt_pat};

pub fn fmt_decl(decl: &Decl, indent: usize) -> String {
    match decl {
        Decl::Fn(f)           => fmt_fn(f, indent),
        Decl::Type(t)         => fmt_type_decl(t, indent),
        Decl::Val(v)          => fmt_val(v, indent),
        Decl::Var(v)          => fmt_var(v, indent),
        Decl::Trait(t)        => fmt_trait(t, indent),
        Decl::Impl(i)         => fmt_impl(i, indent),
        Decl::Migration(m)    => fmt_migration(m, indent),
        Decl::Test(t)         => fmt_test_decl("test", &t.name, &[], &t.body.node, indent),
        Decl::Property(p)     => fmt_test_decl("property", &p.name, &p.params, &p.body.node, indent),
        Decl::DbTest(d)       => fmt_test_decl("dbTest", &d.name, &[], &d.body.node, indent),
        Decl::StateMachine(sm) => fmt_statemachine(sm, indent),
        Decl::Validator(v)      => fmt_validator(v, indent),
        Decl::Constraint(c)     => fmt_constraint(c),
        Decl::Temporal(t)       => fmt_temporal(t),
        Decl::RuleTest(rt)      => fmt_rule_test(rt, indent),
        Decl::ValidatorTest(vt) => fmt_validator_test(vt, indent),
        Decl::View(v)           => fmt_view(v, indent),
        Decl::Form(f)           => fmt_form(f, indent),
        Decl::UiGenerate(g)     => fmt_ui_generate(g, indent),
        Decl::Import(i)         => format!("import {}", i.path.join(".")),
    }
}

// ------------------------------------------------------------------ //
// fn
// ------------------------------------------------------------------ //

pub fn fmt_fn(f: &FnDecl, indent: usize) -> String {
    let mut s = String::new();
    if f.is_pub   { s.push_str("pub "); }
    if f.is_async { s.push_str("async "); }
    s.push_str("fn ");
    s.push_str(&f.name.node);
    s.push_str(&fmt_type_params(&f.type_params));
    s.push('(');
    let params: Vec<String> = f.params.iter().map(fmt_fn_param).collect();
    s.push_str(&params.join(", "));
    s.push(')');
    if let Some(ret) = &f.ret_ty {
        s.push_str(": ");
        s.push_str(&fmt_type(&ret.node, indent));
    }
    s.push_str(&fmt_effects(&f.effects));
    // `extern "C"` declarations have no body; wrap the signature in its own block
    // so the output round-trips back to a valid extern declaration.
    if f.is_extern {
        return format!("extern \"C\" {{\n{}{}\n{}}}", ind(indent + 1), s, ind(indent));
    }
    if let Some(body) = &f.body {
        let body_str = fmt_expr(&body.node, indent);
        let inline = format!("{} = {}", s, body_str);
        if inline.len() <= 80 {
            s = inline;
        } else {
            s.push_str(" =\n");
            s.push_str(&ind(indent + 1));
            s.push_str(&body_str);
        }
    }
    s
}

fn fmt_fn_param(p: &FnParam) -> String {
    let ty = fmt_type(&p.ty.node, 0);
    if let Some(def) = &p.default {
        format!("{}: {} = {}", p.name.node, ty, fmt_expr(&def.node, 0))
    } else {
        format!("{}: {}", p.name.node, ty)
    }
}

// ------------------------------------------------------------------ //
// type
// ------------------------------------------------------------------ //

fn fmt_type_decl(t: &TypeDecl, indent: usize) -> String {
    let pub_str = if t.is_pub { "pub " } else { "" };
    let params  = fmt_type_params(&t.type_params);
    // `@valueObject`/`@aggregate` (BACKLOG item 149) — each on its own line
    // immediately before `type`, matching how they're written in source.
    let annotation_prefix: String = t.annotations.iter().map(|a| format!("@{a}\n")).collect();
    let body = match &t.body {
        TypeBody::Alias(te) =>
            format!("{}type {}{} = {}", pub_str, t.name.node, params, fmt_type(&te.node, indent)),

        TypeBody::Record(rec) => {
            let fields: Vec<String> = rec.fields.iter().map(|f| {
                let opt = if f.optional { "?" } else { "" };
                format!("{}    {}{}: {}", ind(indent), f.name.node, opt, fmt_type(&f.ty.node, indent + 1))
            }).collect();
            let computed: Vec<String> = rec.computed.iter().map(|c| {
                format!("{}    computed {}: {} = {}", ind(indent), c.name.node,
                    fmt_type(&c.ty.node, indent + 1), fmt_expr(&c.body.node, indent + 1))
            }).collect();
            // In-body `fn` methods (BACKLOG item 150) — printed from
            // `rec.methods`'s own retained, un-desugared originals (BACKLOG
            // item 175), not from the synthesized trailing `ImplDecl`
            // (`fmt_module` skips that entirely), so a method stays inside
            // the `type { ... }` body it was written in instead of
            // reappearing as a separate top-level `impl` block.
            let methods: Vec<String> = rec.methods.iter().map(|m| {
                format!("{}    {}", ind(indent), fmt_fn(m, indent + 1))
            }).collect();
            let all: Vec<String> = fields.into_iter().chain(computed).chain(methods).collect();
            format!("{}type {}{} = {{\n{}\n{}}}", pub_str, t.name.node, params, all.join("\n"), ind(indent))
        }

        TypeBody::Sum(variants) => {
            let vs: Vec<String> = variants.iter().map(|v| {
                if v.fields.is_empty() {
                    format!("{}    | {}", ind(indent), v.name.node)
                } else {
                    let fs: Vec<String> = v.fields.iter().map(|f| {
                        if let Some(n) = &f.name {
                            format!("{}: {}", n.node, fmt_type(&f.ty.node, indent + 1))
                        } else {
                            fmt_type(&f.ty.node, indent + 1)
                        }
                    }).collect();
                    format!("{}    | {}({})", ind(indent), v.name.node, fs.join(", "))
                }
            }).collect();
            format!("{}type {}{} =\n{}", pub_str, t.name.node, params, vs.join("\n"))
        }
    };
    format!("{annotation_prefix}{body}")
}

// ------------------------------------------------------------------ //
// val / var
// ------------------------------------------------------------------ //

fn fmt_val(v: &ValDecl, indent: usize) -> String {
    let pub_str = if v.is_pub { "pub " } else { "" };
    let pat = fmt_pat(&v.pattern.node, indent);
    let ty_str = v.ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
    format!("{}val {}{} = {}", pub_str, pat, ty_str, fmt_expr(&v.value.node, indent))
}

fn fmt_var(v: &VarDecl, indent: usize) -> String {
    let pub_str = if v.is_pub { "pub " } else { "" };
    let ty_str = v.ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
    format!("{}var {}{} = {}", pub_str, v.name.node, ty_str, fmt_expr(&v.value.node, indent))
}

// ------------------------------------------------------------------ //
// trait / impl
// ------------------------------------------------------------------ //

fn fmt_trait(t: &TraitDecl, indent: usize) -> String {
    let pub_str = if t.is_pub { "pub " } else { "" };
    let params  = fmt_type_params(&t.type_params);
    let methods: Vec<String> = t.methods.iter()
        .map(|m| format!("{}{}", ind(indent + 1), fmt_fn(m, indent + 1)))
        .collect();
    format!("{}trait {}{} {{\n{}\n{}}}", pub_str, t.name.node, params, methods.join("\n\n"), ind(indent))
}

fn fmt_impl(i: &ImplDecl, indent: usize) -> String {
    let trait_str = i.trait_path.as_ref()
        .map(|p| format!("{} for ", p.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".")))
        .unwrap_or_default();
    let type_str  = i.type_path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
    let params    = fmt_type_params(&i.type_params);
    let methods: Vec<String> = i.methods.iter()
        .map(|m| format!("{}{}", ind(indent + 1), fmt_fn(m, indent + 1)))
        .collect();
    format!("impl{} {}{}{} {{\n{}\n{}}}", params, trait_str, type_str, "", methods.join("\n\n"), ind(indent))
}

// ------------------------------------------------------------------ //
// migration
// ------------------------------------------------------------------ //

fn fmt_migration(m: &MigrationDecl, indent: usize) -> String {
    let desc = m.description.as_deref().map(|d| format!(" // {}", d)).unwrap_or_default();
    let up_ops: Vec<String> = m.up.iter()
        .map(|op| format!("{}{}", ind(indent + 2), fmt_migration_op(op)))
        .collect();
    let down_ops: Vec<String> = m.down.iter()
        .map(|op| format!("{}{}", ind(indent + 2), fmt_migration_op(op)))
        .collect();
    format!(
        "migration \"{}\" {{{}\n{}up {{\n{}\n{}}}\n{}down {{\n{}\n{}}}{}}}",
        m.name, desc,
        ind(indent + 1), up_ops.join("\n"), ind(indent + 1),
        ind(indent + 1), down_ops.join("\n"), ind(indent + 1),
        ind(indent),
    )
}

fn fmt_migration_op(op: &MigrationOp) -> String {
    match op {
        MigrationOp::CreateTable { name, columns, .. } => {
            let cols: Vec<String> = columns.iter().map(fmt_column_def).collect();
            format!("createTable {} {{\n{}\n}}", name, cols.join("\n"))
        }
        MigrationOp::DropTable   { name, .. }       => format!("dropTable {}", name),
        MigrationOp::AlterTable  { name, ops, .. }  => {
            let clauses: Vec<String> = ops.iter().map(fmt_alter_op).collect();
            format!("alterTable {} {{\n{}\n}}", name, clauses.join("\n"))
        }
        MigrationOp::CreateIndex { name, table, columns, .. } =>
            format!("createIndex {} on {} ({})", name, table, columns.join(", ")),
        MigrationOp::DropIndex   { name, .. }       => format!("dropIndex {}", name),
        MigrationOp::RawSql      { sql, .. }         => format!("sql \"{}\"", sql.replace('"', "\\\"")),
    }
}

fn fmt_column_def(col: &ColumnDef) -> String {
    let ty = fmt_type(&col.ty.node, 0);
    let mut flags = Vec::new();
    if col.primary_key { flags.push("primaryKey"); }
    if !col.nullable   { flags.push("notNull"); }
    if col.unique      { flags.push("unique"); }
    let flag_str = if flags.is_empty() { String::new() } else { format!(" {}", flags.join(" ")) };
    format!("    {} {}{}", col.name, ty, flag_str)
}

fn fmt_alter_op(op: &AlterOp) -> String {
    match op {
        AlterOp::AddColumn    { def }                      => format!("    addColumn {}", fmt_column_def(def)),
        AlterOp::DropColumn   { name, .. }                 => format!("    dropColumn {}", name),
        AlterOp::AddForeignKey{ column, references, on_delete, .. } =>
            format!("    addForeignKey {} references {} onDelete {}", column, references, fmt_fk_action(on_delete)),
    }
}

fn fmt_fk_action(a: &FkAction) -> &'static str {
    match a {
        FkAction::Cascade  => "Cascade",
        FkAction::SetNull  => "SetNull",
        FkAction::Restrict => "Restrict",
        FkAction::NoAction => "NoAction",
    }
}

// ------------------------------------------------------------------ //
// test / property / dbTest
// ------------------------------------------------------------------ //

fn fmt_test_decl(kw: &str, name: &str, params: &[FnParam], body: &certo_ast::expr::Expr, indent: usize) -> String {
    let params_str = if params.is_empty() {
        String::new()
    } else {
        format!("({})", params.iter().map(fmt_fn_param).collect::<Vec<_>>().join(", "))
    };
    format!("{} \"{}\"{} {{\n{}{}\n{}}}", kw, name, params_str, ind(indent + 1), fmt_expr(body, indent + 1), ind(indent))
}

// ------------------------------------------------------------------ //
// statemachine / validator
// ------------------------------------------------------------------ //

fn fmt_statemachine(sm: &StateMachineDecl, indent: usize) -> String {
    let states: Vec<String> = sm.states.iter().map(|s| format!("{}    state {}", ind(indent), s.node)).collect();
    let transitions: Vec<String> = sm.transitions.iter().map(|t| {
        let params: Vec<String> = t.params.iter().map(|p| format!("{}: {}", p.name.node, fmt_type(&p.ty.node, 0))).collect();
        format!("{}    transition {} -{}-{}({})", ind(indent), t.from.node, t.event.node, t.to.node, params.join(", "))
    }).collect();
    let mut lines = Vec::new();
    lines.extend(states);
    lines.extend(transitions);
    format!("statemachine {} {{\n{}\n{}}}", sm.name.node, lines.join("\n"), ind(indent))
}

// BACKLOG item 279 — previously formatted as `validator V for ... errors
// ... { rule active { ... } }`, discarding the real `for`/`errors` types
// and every rule's own `after`/`overrides`/`priority`/`require`/`else`
// body on every `certo fmt` run. Now round-trips the full decl.
fn fmt_validator(v: &ValidatorDecl, indent: usize) -> String {
    let pub_ = if v.is_pub { "pub " } else { "" };
    let entity_ty = fmt_type(&v.entity.node, indent);
    let errors_ty = fmt_type(&v.errors.node, indent);
    let trigger_str = v.trigger.as_ref()
        .map(|t| format!(" {}", fmt_trigger(t)))
        .unwrap_or_default();

    let mut parts: Vec<String> = Vec::new();
    if !v.context.is_empty() {
        parts.push(fmt_context_block(&v.context, indent));
    }
    parts.extend(v.rules.iter().map(|r| fmt_rule_decl(r, indent)));

    format!(
        "{pub_}validator {} for {entity_ty} errors {errors_ty}{trigger_str} {{\n{}\n{}}}",
        v.name.node, parts.join("\n"), ind(indent),
    )
}

fn fmt_trigger(t: &TriggerDecl) -> String {
    let op = match t.op {
        TriggerOp::Insert => "Insert",
        TriggerOp::Update => "Update",
    };
    let cond = t.condition.as_ref().map(|c| {
        let cond_op = match c.op {
            TriggerCondOp::Eq    => "==",
            TriggerCondOp::NotEq => "!=",
        };
        format!(" when {} {} {}", c.field.node, cond_op, fmt_expr(&c.value.node, 0))
    }).unwrap_or_default();
    format!("trigger on {op}{cond}")
}

fn fmt_context_block(fields: &[ContextField], indent: usize) -> String {
    let body_indent = ind(indent + 1);
    let field_indent = ind(indent + 2);
    let fields_str: Vec<String> = fields.iter().map(|f| {
        let loaded = f.loaded_by.as_ref()
            .map(|e| format!(" loaded by {}", fmt_expr(&e.node, indent + 2)))
            .unwrap_or_default();
        format!("{field_indent}{}: {}{loaded}", f.name.node, fmt_type(&f.type_ref.node, indent + 2))
    }).collect();
    format!("{body_indent}context {{\n{}\n{body_indent}}}", fields_str.join("\n"))
}

fn fmt_rule_decl(r: &RuleDecl, indent: usize) -> String {
    let body_indent = ind(indent + 1);
    let clause_indent = ind(indent + 2);
    let mut clauses: Vec<String> = r.after.iter()
        .map(|a| format!("{clause_indent}after {}", a.node))
        .collect();
    if let Some(o) = &r.overrides {
        clauses.push(format!("{clause_indent}overrides {}", o.node));
    }
    if let Some(p) = r.priority {
        clauses.push(format!("{clause_indent}priority {p}"));
    }
    clauses.push(format!("{clause_indent}require {}", fmt_expr(&r.require.node, indent + 2)));
    clauses.push(format!("{clause_indent}else {}", fmt_expr(&r.else_.node, indent + 2)));
    format!("{body_indent}rule {} {{\n{}\n{body_indent}}}", r.name.node, clauses.join("\n"))
}

fn fmt_constraint(c: &ConstraintDecl) -> String {
    let pub_ = if c.is_pub { "pub " } else { "" };
    format!("{}constraint {} = {}", pub_, c.name.node, fmt_expr(&c.body.node, 0))
}

fn fmt_temporal(t: &TemporalDecl) -> String {
    let pub_ = if t.is_pub { "pub " } else { "" };
    format!("{}temporal {} = {}", pub_, t.name.node, fmt_expr(&t.body.node, 0))
}

// BACKLOG item 222 — previously formatted both block kinds as a literal
// `{ ... }` placeholder, destroying the real `entity`/`context`/`expect`
// body on every `certo fmt` run instead of round-tripping it (unlike every
// other block-bodied decl kind in this file). Mirrors `fmt_test_decl`'s own
// `"kw \"name\" { ... }"` shape.
fn fmt_rule_test(rt: &RuleTestDecl, indent: usize) -> String {
    let path: Vec<_> = rt.validator.iter().map(|s| s.node.as_str()).collect();
    fmt_test_fixture_block(&format!("ruleTest {}", path.join(".")), &rt.label, &rt.entity.node, &rt.context.node, &rt.expect, indent)
}

fn fmt_validator_test(vt: &ValidatorTestDecl, indent: usize) -> String {
    fmt_test_fixture_block(&format!("validatorTest {}", vt.validator.node), &vt.label, &vt.entity.node, &vt.context.node, &vt.expect, indent)
}

fn fmt_test_fixture_block(
    header: &str, label: &str,
    entity: &certo_ast::expr::Expr, context: &certo_ast::expr::Expr,
    expect: &TestExpectation, indent: usize,
) -> String {
    let body_indent = ind(indent + 1);
    let expect_str = match expect {
        TestExpectation::Pass => "pass".to_string(),
        TestExpectation::Fail { with: None } => "fail".to_string(),
        TestExpectation::Fail { with: Some(e) } => format!("fail with {}", fmt_expr(&e.node, indent + 1)),
    };
    format!(
        "{header} \"{label}\" {{\n\
         {body_indent}entity: {entity}\n\
         {body_indent}context: {context}\n\
         {body_indent}expect: {expect}\n\
         {close}}}",
        entity = fmt_expr(entity, indent + 1),
        context = fmt_expr(context, indent + 1),
        expect = expect_str,
        close = ind(indent),
    )
}

// ------------------------------------------------------------------ //
// view / form  (stub — UI nodes)
// ------------------------------------------------------------------ //

fn fmt_view(v: &ViewDecl, indent: usize) -> String {
    format!("view {} {{\n{}    // ...\n{}}}", v.name.node, ind(indent), ind(indent))
}

fn fmt_form(f: &FormDecl, indent: usize) -> String {
    let target = if f.target.segments.is_empty() {
        String::new()
    } else {
        format!(" -> {}", f.target.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."))
    };
    if f.fields.is_empty() && f.pk.is_none() && f.on_submit.is_none() && f.on_success.is_none() {
        return format!("form {}{} {{}}", f.name.node, target);
    }
    let body_ind = format!("{}    ", ind(indent));
    let mut lines = Vec::new();
    for field in &f.fields {
        // BACKLOG item 166 — a field with any nested-only metadata
        // (label/placeholder/options/rows) round-trips through the real
        // `field NAME { ... }` block, not the flat shorthand, or that
        // metadata would be silently dropped on every `certo fmt`. Also
        // required whenever `field_type` is `None` (e.g. an empty `field
        // name {}` block): the flat shorthand's own grammar requires a
        // type after the colon (`cur.expect(&Token::Colon)?;
        // parse_expr(cur)?`, no `None` case exists there at all), so
        // printing `name: ` with nothing after the colon would emit
        // invalid, unparseable source. A field using only the flat
        // shorthand keeps printing exactly as before — idempotent for
        // every form that predates this item.
        let needs_nested_form = field.label.is_some() || field.placeholder.is_some()
            || field.options.is_some() || field.rows.is_some() || field.field_type.is_none();
        if needs_nested_form {
            lines.push(format!("{body_ind}{}", fmt_form_field(field, indent + 1)));
        } else {
            let ty = field.field_type.as_ref().map(|e| fmt_expr(&e.node, indent + 1)).unwrap_or_default();
            lines.push(format!("{body_ind}{}: {ty}", field.name.node));
        }
    }
    if let Some(pk) = &f.pk {
        lines.push(format!("{body_ind}pk: {pk}"));
    }
    if let Some(on_submit) = &f.on_submit {
        lines.push(format!("{body_ind}onSubmit: {}", fmt_expr(&on_submit.node, indent + 1)));
    }
    if let Some(on_success) = &f.on_success {
        lines.push(format!("{body_ind}onSuccess: {}", fmt_expr(&on_success.node, indent + 1)));
    }
    format!("form {}{} {{\n{}\n{}}}", f.name.node, target, lines.join("\n"), ind(indent))
}

/// Print one `field NAME { ... }` block — `field_indent` is the indent
/// level the `field NAME {` line itself sits at (the same level a flat
/// `name: Type` field line would use).
fn fmt_form_field(field: &FormField, field_indent: usize) -> String {
    let inner_ind = format!("{}    ", ind(field_indent));
    let mut lines = Vec::new();
    if let Some(label) = &field.label {
        lines.push(format!("{inner_ind}label: \"{label}\""));
    }
    if let Some(placeholder) = &field.placeholder {
        lines.push(format!("{inner_ind}placeholder: \"{placeholder}\""));
    }
    if let Some(ty) = &field.field_type {
        lines.push(format!("{inner_ind}type: {}", fmt_expr(&ty.node, field_indent + 1)));
    }
    if let Some(options) = &field.options {
        lines.push(format!("{inner_ind}options: {}", fmt_expr(&options.node, field_indent + 1)));
    }
    if let Some(rows) = field.rows {
        lines.push(format!("{inner_ind}rows: {rows}"));
    }
    if lines.is_empty() {
        format!("field {} {{}}", field.name.node)
    } else {
        format!("field {} {{\n{}\n{}}}", field.name.node, lines.join("\n"), ind(field_indent))
    }
}

fn fmt_ui_generate(g: &certo_ast::decl::UiGenerateDecl, indent: usize) -> String {
    let mut out = format!("@ui.generate({}) {{\n", g.type_name.node);
    if let Some(title) = &g.title {
        out.push_str(&format!("{}    title: \"{}\"\n", ind(indent), title));
    }
    if !g.columns.is_empty() {
        out.push_str(&format!("{}    list: {{ columns: [{}] }}\n", ind(indent), g.columns.join(", ")));
    }
    out.push_str(&format!("{}}}", ind(indent)));
    out
}
