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
        Decl::Test(t)         => fmt_test_decl("test", &t.name, &t.body.node, indent),
        Decl::Property(p)     => fmt_test_decl("property", &p.name, &p.body.node, indent),
        Decl::DbTest(d)       => fmt_test_decl("dbTest", &d.name, &d.body.node, indent),
        Decl::StateMachine(sm) => fmt_statemachine(sm, indent),
        Decl::Validator(v)      => fmt_validator(v, indent),
        Decl::Constraint(c)     => fmt_constraint(c),
        Decl::Temporal(t)       => fmt_temporal(t),
        Decl::RuleTest(rt)      => fmt_rule_test(rt),
        Decl::ValidatorTest(vt) => fmt_validator_test(vt),
        Decl::View(v)           => fmt_view(v, indent),
        Decl::Form(f)           => fmt_form(f, indent),
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
    match &t.body {
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
            let all: Vec<String> = fields.into_iter().chain(computed).collect();
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
    }
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

fn fmt_test_decl(kw: &str, name: &str, body: &certo_ast::expr::Expr, indent: usize) -> String {
    format!("{} \"{}\" {{\n{}{}\n{}}}", kw, name, ind(indent + 1), fmt_expr(body, indent + 1), ind(indent))
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

fn fmt_validator(v: &ValidatorDecl, indent: usize) -> String {
    let pub_ = if v.is_pub { "pub " } else { "" };
    let rules: Vec<String> = v.rules.iter()
        .map(|r| format!("{}    rule {} {{ ... }}", ind(indent), r.name.node))
        .collect();
    format!("{}validator {} for ... errors ... {{\n{}\n{}}}", pub_, v.name.node, rules.join("\n"), ind(indent))
}

fn fmt_constraint(c: &ConstraintDecl) -> String {
    let pub_ = if c.is_pub { "pub " } else { "" };
    format!("{}constraint {} = {}", pub_, c.name.node, fmt_expr(&c.body.node, 0))
}

fn fmt_temporal(t: &TemporalDecl) -> String {
    let pub_ = if t.is_pub { "pub " } else { "" };
    format!("{}temporal {} = {}", pub_, t.name.node, fmt_expr(&t.body.node, 0))
}

fn fmt_rule_test(rt: &RuleTestDecl) -> String {
    let path: Vec<_> = rt.validator.iter().map(|s| s.node.as_str()).collect();
    format!("ruleTest {} \"{}\" {{ ... }}", path.join("."), rt.label)
}

fn fmt_validator_test(vt: &ValidatorTestDecl) -> String {
    format!("validatorTest {} \"{}\" {{ ... }}", vt.validator.node, vt.label)
}

// ------------------------------------------------------------------ //
// view / form  (stub — UI nodes)
// ------------------------------------------------------------------ //

fn fmt_view(v: &ViewDecl, indent: usize) -> String {
    format!("view {} {{\n{}    // ...\n{}}}", v.name.node, ind(indent), ind(indent))
}

fn fmt_form(f: &FormDecl, indent: usize) -> String {
    format!("form {} {{\n{}    // ...\n{}}}", f.name.node, ind(indent), ind(indent))
}
