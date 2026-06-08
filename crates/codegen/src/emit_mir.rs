use std::fmt::Write as FmtWrite;
use certo_mir::{MirFn, MirStmt, Rvalue, Operand, MirConst, Terminator, AggregateKind, BlockId};
use certo_hir::{BinOp, UnOp};
use certo_typeck::Ty;
use crate::ty_to_c::{ty_to_c, ret_ty_to_c};

/// Emit a single MIR function as a C function definition with an optional prefix
/// (e.g. `"CERTO_EXPORT "` for shared library builds).
pub fn emit_fn_with_prefix(f: &MirFn, prefix: &str, out: &mut String) {
    // Temporarily intercept the signature line to inject the prefix.
    let mut body = String::new();
    emit_fn_inner(f, prefix, &mut body);
    out.push_str(&body);
}

/// Emit a single MIR function as a C function definition.
pub fn emit_fn(f: &MirFn, out: &mut String) {
    emit_fn_inner(f, "", out);
}

fn emit_fn_inner(f: &MirFn, prefix: &str, out: &mut String) {
    // Determine return type from the _ret local (index 0).
    let ret_ty = f.locals.first().map(|l| &l.ty).unwrap_or(&Ty::Unit);
    let ret_c  = ret_ty_to_c(ret_ty);

    // Params are locals 1..params.len()+1 (the first local is the return slot).
    // We reconstruct params by inspecting the HIR fn — here we just emit all
    // non-ret locals that appear to be params (name not starting with `_`).
    let params: Vec<String> = f.locals.iter().skip(1)
        .filter(|l| !l.name.starts_with('_'))
        .map(|l| format!("{} {}", ty_to_c(&l.ty), local_name(l.id)))
        .collect();

    let param_str = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    writeln!(out, "{}{} {}({}) {{", prefix, ret_c, c_fn_name(&f.name), param_str).unwrap();

    // Declare all temporaries (locals starting with `_`).
    for local in &f.locals {
        if local.name.starts_with('_') || local.id == 0 {
            // ret slot declared as the return value type
            if local.id == 0 {
                if !matches!(ret_ty, Ty::Unit) {
                    writeln!(out, "    {} {};", ty_to_c(&local.ty), local_name(local.id)).unwrap();
                }
            } else {
                writeln!(out, "    {} {};", ty_to_c(&local.ty), local_name(local.id)).unwrap();
            }
        }
    }

    // Emit each basic block as a labeled section.
    for bb in &f.blocks {
        writeln!(out, "  bb{}:", bb.id).unwrap();
        for stmt in &bb.stmts {
            emit_stmt(stmt, out);
        }
        if let Some(term) = &bb.terminator {
            emit_terminator(term, ret_ty, out);
        }
    }

    writeln!(out, "}}").unwrap();
}

fn emit_stmt(stmt: &MirStmt, out: &mut String) {
    let MirStmt::Assign { dest, rvalue } = stmt;
    let lhs = local_name(*dest);
    match rvalue {
        Rvalue::Use(op) => {
            writeln!(out, "    {} = {};", lhs, emit_operand(op)).unwrap();
        }
        Rvalue::BinOp { op, lhs: l, rhs: r } => {
            writeln!(out, "    {} = {};", lhs, emit_binop(op, l, r)).unwrap();
        }
        Rvalue::UnOp { op, arg } => {
            let sym = match op { UnOp::Neg => "-", UnOp::Not => "!" };
            writeln!(out, "    {} = {}({});", lhs, sym, emit_operand(arg)).unwrap();
        }
        Rvalue::Call { func, args } => {
            let args_str = args.iter().map(emit_operand).collect::<Vec<_>>().join(", ");
            writeln!(out, "    {} = {}({});", lhs, emit_operand(func), args_str).unwrap();
        }
        Rvalue::Aggregate(kind, ops) => {
            match kind {
                AggregateKind::Tuple => {
                    // Emit as struct literal initialiser.
                    let fields = ops.iter().enumerate()
                        .map(|(i, o)| format!(".f{} = {}", i, emit_operand(o)))
                        .collect::<Vec<_>>().join(", ");
                    writeln!(out, "    {} = (typeof({})){{ {} }};", lhs, lhs, fields).unwrap();
                }
                AggregateKind::Record(names) => {
                    let fields = names.iter().zip(ops.iter())
                        .map(|(n, o)| format!(".{} = {}", n, emit_operand(o)))
                        .collect::<Vec<_>>().join(", ");
                    writeln!(out, "    {} = (typeof({})){{ {} }};", lhs, lhs, fields).unwrap();
                }
                AggregateKind::Array => {
                    let elems = ops.iter().map(emit_operand).collect::<Vec<_>>().join(", ");
                    writeln!(out, "    {} = certo_list_new({}, (void*[]){{{}}});",
                        lhs, ops.len(), elems).unwrap();
                }
            }
        }
    }
}

fn emit_terminator(term: &Terminator, ret_ty: &Ty, out: &mut String) {
    match term {
        Terminator::Goto(bb) => {
            writeln!(out, "    goto bb{};", bb).unwrap();
        }
        Terminator::If { cond, true_bb, false_bb } => {
            writeln!(out, "    if ({}) goto bb{}; else goto bb{};",
                emit_operand(cond), true_bb, false_bb).unwrap();
        }
        Terminator::Return(op) => {
            if matches!(ret_ty, Ty::Unit) {
                writeln!(out, "    return;").unwrap();
            } else {
                writeln!(out, "    return {};", emit_operand(op)).unwrap();
            }
        }
        Terminator::Unreachable => {
            writeln!(out, "    __builtin_unreachable();").unwrap();
        }
        Terminator::Call { func, args, dest, next } => {
            let args_str = args.iter().map(emit_operand).collect::<Vec<_>>().join(", ");
            writeln!(out, "    {} = {}({});", local_name(*dest), emit_operand(func), args_str).unwrap();
            writeln!(out, "    goto bb{};", next).unwrap();
        }
        Terminator::Switch { discr, targets, otherwise } => {
            writeln!(out, "    switch ({}) {{", emit_operand(discr)).unwrap();
            for (target, bb) in targets {
                let val = match target {
                    certo_mir::SwitchTarget::Int(n)  => n.to_string(),
                    certo_mir::SwitchTarget::Bool(b) => if *b { "1".into() } else { "0".into() },
                };
                writeln!(out, "      case {}: goto bb{};", val, bb).unwrap();
            }
            writeln!(out, "      default: goto bb{};", otherwise).unwrap();
            writeln!(out, "    }}").unwrap();
        }
    }
}

fn emit_operand(op: &Operand) -> String {
    match op {
        Operand::Local(id)  => local_name(*id),
        Operand::Global(n)  => c_fn_name(n),
        Operand::Const(c)   => emit_const(c),
    }
}

fn emit_const(c: &MirConst) -> String {
    match c {
        MirConst::Int(n)     => n.to_string(),
        MirConst::Float(f)   => format!("{:.}", f),
        MirConst::Decimal(s) => format!("CERTO_DECIMAL(\"{}\")", s),
        MirConst::Bool(b)    => if *b { "true".into() } else { "false".into() },
        MirConst::Str(s)     => format!("CERTO_STR(\"{}\")", escape_str(s)),
        MirConst::Uuid(u)    => format!("CERTO_UUID(\"{}\")", u),
        MirConst::Unit       => "CERTO_UNIT".into(),
    }
}

fn emit_binop(op: &BinOp, l: &Operand, r: &Operand) -> String {
    let lhs = emit_operand(l);
    let rhs = emit_operand(r);
    match op {
        BinOp::Add  => format!("({} + {})", lhs, rhs),
        BinOp::Sub  => format!("({} - {})", lhs, rhs),
        BinOp::Mul  => format!("({} * {})", lhs, rhs),
        BinOp::Div  => format!("({} / {})", lhs, rhs),
        BinOp::Rem  => format!("({} % {})", lhs, rhs),
        BinOp::Pow  => format!("certo_pow({}, {})", lhs, rhs),
        BinOp::Eq   => format!("({} == {})", lhs, rhs),
        BinOp::NotEq => format!("({} != {})", lhs, rhs),
        BinOp::Lt   => format!("({} < {})", lhs, rhs),
        BinOp::LtEq => format!("({} <= {})", lhs, rhs),
        BinOp::Gt   => format!("({} > {})", lhs, rhs),
        BinOp::GtEq => format!("({} >= {})", lhs, rhs),
        BinOp::And  => format!("({} && {})", lhs, rhs),
        BinOp::Or   => format!("({} || {})", lhs, rhs),
        BinOp::NullCoalesce => format!("certo_coalesce({}, {})", lhs, rhs),
    }
}

fn local_name(id: u32) -> String {
    format!("_l{}", id)
}

pub fn c_fn_name(name: &str) -> String {
    // Convert camelCase to snake_case so Certo names match C stdlib conventions.
    let snake = camel_to_snake(name);
    format!("certo_{}", snake.replace('.', "_").replace('-', "_"))
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t")
}
