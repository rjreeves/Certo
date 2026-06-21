use std::fmt::Write as FmtWrite;
use certo_mir::{MirFn, MirStmt, MirLocalDecl, Rvalue, Operand, MirConst, Terminator, AggregateKind, BasicBlock};
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

/// Emit the spawn-wrapper struct typedefs and worker functions for all SpawnCall sites
/// in the given MirFn. Must be emitted BEFORE the function that contains the spawn calls.
pub fn emit_spawn_preamble(f: &MirFn, out: &mut String) {
    let fn_safe = c_fn_name(&f.name);
    for bb in &f.blocks {
        emit_spawn_preamble_for_block(bb, &fn_safe, out);
    }
}

fn emit_spawn_preamble_for_block(bb: &BasicBlock, fn_safe: &str, out: &mut String) {
    for stmt in &bb.stmts {
        if let MirStmt::Assign { rvalue: Rvalue::SpawnCall { spawn_id, func, args }, .. } = stmt {
            let key    = format!("{fn_safe}_{spawn_id}");
            let n_args = args.len();

            // Typed ctx struct: fixed header (thr + result) followed by arg slots.
            writeln!(out, "typedef struct {{").unwrap();
            writeln!(out, "    pthread_t __thr; int64_t __result;").unwrap();
            if n_args > 0 {
                writeln!(out, "    int64_t __args[{n_args}];").unwrap();
            }
            writeln!(out, "}} __certo_spawn_ctx_{key}_t;").unwrap();

            // Worker function called by pthread_create.
            let args_str = (0..n_args)
                .map(|i| format!("_c->__args[{i}]"))
                .collect::<Vec<_>>()
                .join(", ");
            let func_call = match func {
                Operand::Global(name) => format!("{}({args_str})", c_fn_name(name)),
                Operand::Local(id)    => format!("((int64_t(*)())(intptr_t){})({})", local_name(*id), args_str),
                Operand::Const(_)     => "0".into(),
            };
            writeln!(out, "static void* __certo_spawn_worker_{key}(void* _raw) {{").unwrap();
            writeln!(out, "    __certo_spawn_ctx_{key}_t* _c = (__certo_spawn_ctx_{key}_t*)_raw;").unwrap();
            writeln!(out, "    _c->__result = (int64_t)(intptr_t)({func_call});").unwrap();
            writeln!(out, "    return NULL;").unwrap();
            writeln!(out, "}}").unwrap();
        }
    }
}

fn emit_fn_inner(f: &MirFn, prefix: &str, out: &mut String) {
    // Determine return type from the _ret local (index 0).
    let ret_ty = f.locals.first().map(|l| &l.ty).unwrap_or(&Ty::Unit);
    let ret_c  = ret_ty_to_c(ret_ty);

    // Params are locals 1..=param_count (the first local is the return slot).
    let params: Vec<String> = f.locals.iter().skip(1).take(f.param_count)
        .map(|l| format!("{} {}", ty_to_c(&l.ty), local_name(l.id)))
        .collect();

    let param_str = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    writeln!(out, "{}{} {}({}) {{", prefix, ret_c, c_fn_name(&f.name), param_str).unwrap();

    // Declare return slot and all non-param locals as temporaries.
    for local in &f.locals {
        let is_ret   = local.id == 0;
        let is_param = !is_ret && (local.id as usize) <= f.param_count;
        if is_param { continue; }
        if is_ret && matches!(ret_ty, Ty::Unit) { continue; }
        writeln!(out, "    {} {};", ty_to_c(&local.ty), local_name(local.id)).unwrap();
    }

    // Emit each basic block as a labeled section.
    for bb in &f.blocks {
        writeln!(out, "  bb{}:", bb.id).unwrap();
        let fn_safe = c_fn_name(&f.name);
        for stmt in &bb.stmts {
            emit_stmt(stmt, &f.locals, &fn_safe, out);
        }
        if let Some(term) = &bb.terminator {
            emit_terminator(term, ret_ty, &f.locals, out);
        }
    }

    writeln!(out, "}}").unwrap();
}

fn emit_stmt(stmt: &MirStmt, locals: &[MirLocalDecl], fn_safe: &str, out: &mut String) {
    let MirStmt::Assign { dest, rvalue } = stmt;
    let lhs = local_name(*dest);
    let dest_ty = local_ty(*dest, locals);
    match rvalue {
        Rvalue::Use(op) => {
            let expr = cast_for_assignment(dest_ty, emit_operand(op));
            writeln!(out, "    {} = {};", lhs, expr).unwrap();
        }
        Rvalue::BinOp { op, lhs: l, rhs: r } => {
            let expr = emit_binop(op, l, r, locals);
            // NullCoalesce returns void* — cast back to the destination's type.
            if matches!(op, BinOp::NullCoalesce) {
                writeln!(out, "    {} = (__typeof__({})){};", lhs, lhs, expr).unwrap();
            } else {
                writeln!(out, "    {} = {};", lhs, expr).unwrap();
            }
        }
        Rvalue::UnOp { op, arg } => {
            let sym = match op { UnOp::Neg => "-", UnOp::Not => "!" };
            writeln!(out, "    {} = {}({});", lhs, sym, emit_operand(arg)).unwrap();
        }
        Rvalue::Call { func, args } => {
            let args_str = emit_call_args(func, args, locals);
            let expr = cast_for_call_assignment(dest_ty, func, format!("{}({})", emit_operand(func), args_str));
            writeln!(out, "    {} = {};", lhs, expr).unwrap();
        }
        Rvalue::Field { base, field } => {
            writeln!(out, "    {} = {}.{};", lhs, emit_operand(base), field).unwrap();
        }
        Rvalue::Aggregate(kind, ops) => {
            match kind {
                AggregateKind::Tuple => {
                    // Represent tuples as CertoList* — same as arrays.
                    if ops.is_empty() {
                        writeln!(out, "    {} = (void*)0;", lhs).unwrap();
                    } else {
                        let elems = ops.iter().map(|o| format!("(void*)(intptr_t)({})", emit_operand(o))).collect::<Vec<_>>().join(", ");
                        writeln!(out, "    {} = certo_list_of({}, {});", lhs, ops.len(), elems).unwrap();
                    }
                }
                AggregateKind::Record(names) => {
                    let fields = names.iter().zip(ops.iter())
                        .map(|(n, o)| format!(".{} = {}", n, emit_operand(o)))
                        .collect::<Vec<_>>().join(", ");
                    writeln!(out, "    {} = (typeof({})){{ {} }};", lhs, lhs, fields).unwrap();
                }
                AggregateKind::Array => {
                    if ops.is_empty() {
                        writeln!(out, "    {} = certo_list_new_empty();", lhs).unwrap();
                    } else {
                        let elems = ops.iter().map(|o| format!("(void*)({})", emit_operand(o))).collect::<Vec<_>>().join(", ");
                        writeln!(out, "    {} = certo_list_of({}, {});", lhs, ops.len(), elems).unwrap();
                    }
                }
            }
        }
        Rvalue::SpawnCall { spawn_id, func: _, args } => {
            let key = format!("{fn_safe}_{spawn_id}");
            writeln!(out, "    {{ __certo_spawn_ctx_{key}_t* _sc = (__certo_spawn_ctx_{key}_t*)malloc(sizeof(*_sc));").unwrap();
            for (i, arg) in args.iter().enumerate() {
                writeln!(out, "    _sc->__args[{i}] = (int64_t)(intptr_t)({});", emit_operand(arg)).unwrap();
            }
            writeln!(out, "    pthread_create(&_sc->__thr, NULL, __certo_spawn_worker_{key}, _sc);").unwrap();
            writeln!(out, "    {lhs} = (int64_t)(intptr_t)_sc; }}").unwrap();
        }
        Rvalue::JoinTask(task_op) => {
            let task = emit_operand(task_op);
            writeln!(out, "    {{ __certo_task_hdr_t* _th = (__certo_task_hdr_t*)(intptr_t)({task});").unwrap();
            writeln!(out, "    pthread_join(_th->__thr, NULL);").unwrap();
            writeln!(out, "    {lhs} = _th->__result;").unwrap();
            writeln!(out, "    free(_th); }}").unwrap();
        }
    }
}

fn emit_terminator(term: &Terminator, ret_ty: &Ty, locals: &[MirLocalDecl], out: &mut String) {
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
                writeln!(out, "    return 0;").unwrap();
            } else {
                writeln!(out, "    return {};", emit_operand(op)).unwrap();
            }
        }
        Terminator::Unreachable => {
            writeln!(out, "    __builtin_unreachable();").unwrap();
        }
        Terminator::Call { func, args, dest, next } => {
            let args_str = emit_call_args(func, args, locals);
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
        MirConst::Unit       => "0".into(), // Unit locals are int64_t, 0 is compatible
    }
}

fn operand_is_text(op: &Operand, locals: &[MirLocalDecl]) -> bool {
    match op {
        Operand::Local(id) => locals.iter().any(|l| l.id == *id && (matches!(l.ty, Ty::Text) || matches!(&l.ty, Ty::Option(t) if matches!(t.as_ref(), Ty::Text)))),
        Operand::Const(MirConst::Str(_)) => true,
        _ => false,
    }
}

fn operand_ty<'a>(op: &Operand, locals: &'a [MirLocalDecl]) -> Option<&'a Ty> {
    match op {
        Operand::Local(id) => locals.iter().find(|l| l.id == *id).map(|l| &l.ty),
        _ => None,
    }
}

fn local_ty<'a>(id: u32, locals: &'a [MirLocalDecl]) -> Option<&'a Ty> {
    locals.iter().find(|l| l.id == id).map(|l| &l.ty)
}

fn cast_for_assignment(dest_ty: Option<&Ty>, expr: String) -> String {
    match dest_ty {
        Some(Ty::Option(inner)) if matches!(inner.as_ref(), Ty::Text) => format!("(void*)({})", expr),
        _ => expr,
    }
}

fn cast_for_call_assignment(dest_ty: Option<&Ty>, func: &Operand, expr: String) -> String {
    if returns_erased_text_option(func) {
        format!("(void*)({})", expr)
    } else {
        cast_for_assignment(dest_ty, expr)
    }
}

fn returns_erased_text_option(func: &Operand) -> bool {
    match func {
        Operand::Global(name) => matches!(
            name.as_str(),
            "arg" | "readLine" | "readFile" | "getEnv" | "Path.extension"
        ),
        _ => false,
    }
}

fn emit_call_args(func: &Operand, args: &[Operand], locals: &[MirLocalDecl]) -> String {
    let func_name = match func {
        Operand::Global(name) => Some(name.as_str()),
        _ => None,
    };

    args.iter()
        .enumerate()
        .map(|(i, arg)| {
            let expr = emit_operand(arg);
            if func_name == Some("List.push") && i == 1 && operand_is_text(arg, locals) {
                format!("(void*)({})", expr)
            } else {
                expr
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_binop(op: &BinOp, l: &Operand, r: &Operand, locals: &[MirLocalDecl]) -> String {
    let lhs = emit_operand(l);
    let rhs = emit_operand(r);
    match op {
        BinOp::Add  => format!("({} + {})", lhs, rhs),
        BinOp::Sub  => format!("({} - {})", lhs, rhs),
        BinOp::Mul  => format!("({} * {})", lhs, rhs),
        BinOp::Div  => format!("({} / {})", lhs, rhs),
        BinOp::Rem  => format!("({} % {})", lhs, rhs),
        BinOp::Pow  => format!("certo_pow({}, {})", lhs, rhs),
        BinOp::Eq   => {
            if operand_is_text(l, locals) || operand_is_text(r, locals) {
                format!("(certo_text_eq({}, {}))", lhs, rhs)
            } else {
                format!("({} == {})", lhs, rhs)
            }
        }
        BinOp::NotEq => {
            if operand_is_text(l, locals) || operand_is_text(r, locals) {
                format!("(!certo_text_eq({}, {}))", lhs, rhs)
            } else {
                format!("({} != {})", lhs, rhs)
            }
        }
        BinOp::Lt   => format!("({} < {})", lhs, rhs),
        BinOp::LtEq => format!("({} <= {})", lhs, rhs),
        BinOp::Gt   => format!("({} > {})", lhs, rhs),
        BinOp::GtEq => format!("({} >= {})", lhs, rhs),
        BinOp::And  => format!("({} && {})", lhs, rhs),
        BinOp::Or   => format!("({} || {})", lhs, rhs),
        BinOp::NullCoalesce => {
            match operand_ty(l, locals) {
                Some(Ty::Option(inner)) if matches!(inner.as_ref(), Ty::Int) => {
                    format!("(({lhs}) ? __option_unwrap_int((void*)({lhs})) : ({rhs}))")
                }
                Some(Ty::Option(inner)) if matches!(inner.as_ref(), Ty::Float) => {
                    format!("(({lhs}) ? __option_unwrap_float((void*)({lhs})) : ({rhs}))")
                }
                Some(Ty::Option(inner)) if matches!(inner.as_ref(), Ty::Bool) => {
                    format!("(({lhs}) ? __option_unwrap_bool((void*)({lhs})) : ({rhs}))")
                }
                _ => format!("certo_coalesce((void*)({lhs}), (void*)({rhs}))"),
            }
        }
        BinOp::Concat       => format!("certo_text_concat({}, {})", lhs, rhs),
    }
}

fn local_name(id: u32) -> String {
    format!("_l{}", id)
}

pub fn c_fn_name(name: &str) -> String {
    // Sum-type variant tag constant: `__tag__TypeName__VariantName` → `TypeName_VariantName`
    if let Some(rest) = name.strip_prefix("__tag__") {
        return rest.replacen("__", "_", 1);
    }
    // Runtime intrinsics (__ prefix) are emitted verbatim — no certo_ wrapper.
    if name.starts_with("__") { return name.to_string(); }
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
