use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use certo_mir::{MirFn, MirStmt, MirLocalDecl, Rvalue, Operand, MirConst, Terminator, AggregateKind};
use certo_hir::{BinOp, UnOp};
use certo_typeck::Ty;
use crate::ty_to_c::{ty_to_c, ret_ty_to_c, mangle};

// ------------------------------------------------------------------ //
// Concurrency: per-spawn-call-site worker functions
// ------------------------------------------------------------------ //

/// One `spawn`/`parallel` call site: enough to generate a context struct and a
/// thread-worker function for it.
pub struct SpawnSite {
    pub ctx_name:    String,
    pub worker_name: String,
    pub func_c:      String,   // C name of the function to call (e.g. `certo_work`)
    pub arg_tys:     Vec<Ty>,
    pub ret_ty:      Ty,
}

/// Collect the spawn sites of a function in emission order. The names are derived
/// deterministically from the function's C name and the site index, so the body
/// emitter (which recomputes them) stays in sync with the preamble.
pub fn collect_spawn_sites(f: &MirFn) -> Vec<SpawnSite> {
    let fname = c_fn_name(&f.name);
    let mut sites = Vec::new();
    let mut idx = 0u32;
    for bb in &f.blocks {
        for stmt in &bb.stmts {
            if let MirStmt::Assign { rvalue: Rvalue::Spawn { func, args, ret_ty }, .. } = stmt {
                let func_c = match func {
                    Operand::Global(n) => c_fn_name(n),
                    other => emit_operand(other),
                };
                let arg_tys = args.iter().map(|a| operand_ty(a, &f.locals)).collect();
                sites.push(SpawnSite {
                    ctx_name:    format!("__certo_ctx_{}_{}_t", fname, idx),
                    worker_name: format!("__certo_worker_{}_{}", fname, idx),
                    func_c,
                    arg_tys,
                    ret_ty: ret_ty.clone(),
                });
                idx += 1;
            }
        }
    }
    sites
}

/// Emit the context structs, thread workers, and result-accessor typedefs for a
/// set of spawn sites. `emitted_joins` dedupes the per-result-type accessors
/// across the whole module.
pub fn emit_spawn_support(sites: &[SpawnSite], emitted_joins: &mut HashSet<String>, out: &mut String) {
    for s in sites {
        let rc = ret_ty_to_c(&s.ret_ty);
        let rmangle = mangle(&s.ret_ty);
        // Result-accessor: a layout-compatible prefix of every ctx struct with
        // this result type, so `join` can read `.result` without knowing the site.
        if emitted_joins.insert(rmangle.clone()) {
            writeln!(out, "typedef struct {{ __certo_task_hdr_t hdr; {} result; }} __certo_join_{}_t;", rc, rmangle).unwrap();
        }
        // Per-site context struct: header + result + the call arguments.
        writeln!(out, "typedef struct {{").unwrap();
        writeln!(out, "    __certo_task_hdr_t hdr;").unwrap();
        writeln!(out, "    {} result;", rc).unwrap();
        for (i, t) in s.arg_tys.iter().enumerate() {
            writeln!(out, "    {} a{};", ty_to_c(t), i).unwrap();
        }
        writeln!(out, "}} {};", s.ctx_name).unwrap();
        // Worker: unpack the context, call the function, store the result.
        let call_args = (0..s.arg_tys.len()).map(|i| format!("c->a{}", i)).collect::<Vec<_>>().join(", ");
        writeln!(out, "static void* {}(void* _p) {{", s.worker_name).unwrap();
        writeln!(out, "    {}* c = ({}*)_p;", s.ctx_name, s.ctx_name).unwrap();
        writeln!(out, "    c->result = {}({});", s.func_c, call_args).unwrap();
        writeln!(out, "    return 0;").unwrap();
        writeln!(out, "}}").unwrap();
    }
}

fn operand_ty(op: &Operand, locals: &[MirLocalDecl]) -> Ty {
    match op {
        Operand::Local(id) => locals.iter().find(|l| l.id == *id).map(|l| l.ty.clone()).unwrap_or(Ty::Error),
        Operand::Const(MirConst::Int(_))   => Ty::Int,
        Operand::Const(MirConst::Float(_)) => Ty::Float,
        Operand::Const(MirConst::Bool(_))  => Ty::Bool,
        Operand::Const(MirConst::Str(_))   => Ty::Text,
        Operand::Const(MirConst::Unit)     => Ty::Unit,
        _ => Ty::Error,
    }
}

// ------------------------------------------------------------------ //
// Unified value boxing: pack any value into a pointer-sized generic slot
// (list element, tuple element, Option/Result payload) bit-preserving, and
// recover it with its type. This is the single ABI for values crossing into a
// `void*`/`int64` slot — it stops `double` (and other non-int-shaped values)
// being numeric-converted, which silently corrupted them.
// ------------------------------------------------------------------ //

/// Pack `value` (of type `ty`) into a pointer-sized slot, preserving its bits.
fn box_value(value: &str, ty: &Ty) -> String {
    match ty {
        // A double must be bit-cast, never numeric-converted.
        Ty::Float | Ty::Decimal => format!("(void*)__certo_f2i({})", value),
        // Everything else here is already pointer-sized (ints, bool, Text, and
        // handle types like Option/List/Result/Map/Tuple/nullary-enum).
        _ => format!("(void*)(intptr_t)({})", value),
    }
}

/// Recover a value of type `ty` from a pointer-sized slot.
fn unbox_value(slot: &str, ty: &Ty) -> String {
    match ty {
        Ty::Float | Ty::Decimal => format!("__certo_i2f((int64_t)(intptr_t)({}))", slot),
        _ => format!("({})(intptr_t)({})", ty_to_c(ty), slot),
    }
}

/// Emit a single MIR function as a C function definition with an optional prefix
/// (e.g. `"CERTO_EXPORT "` for shared library builds).
pub fn emit_fn_with_prefix(f: &MirFn, prefix: &str, nullary_enums: &HashSet<String>, out: &mut String) {
    // Temporarily intercept the signature line to inject the prefix.
    let mut body = String::new();
    emit_fn_inner(f, prefix, nullary_enums, &mut body);
    out.push_str(&body);
}

fn emit_fn_inner(f: &MirFn, prefix: &str, nullary_enums: &HashSet<String>, out: &mut String) {
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

    // Emit each basic block as a labeled section. `spawn_idx` tracks spawn sites
    // in the same order as collect_spawn_sites so the generated worker/ctx names
    // line up with the preamble.
    let fn_cname = c_fn_name(&f.name);
    let mut spawn_idx = 0u32;
    for bb in &f.blocks {
        writeln!(out, "  bb{}:", bb.id).unwrap();
        for stmt in &bb.stmts {
            emit_stmt(stmt, &f.locals, &fn_cname, &mut spawn_idx, nullary_enums, out);
        }
        if let Some(term) = &bb.terminator {
            emit_terminator(term, ret_ty, &f.locals, out);
        }
    }

    writeln!(out, "}}").unwrap();
}

fn emit_stmt(stmt: &MirStmt, locals: &[MirLocalDecl], fn_cname: &str, spawn_idx: &mut u32, nullary_enums: &HashSet<String>, out: &mut String) {
    let MirStmt::Assign { dest, rvalue } = stmt;
    let lhs = local_name(*dest);
    match rvalue {
        Rvalue::Use(op) => {
            writeln!(out, "    {} = {};", lhs, emit_operand(op)).unwrap();
        }
        Rvalue::BinOp { op, lhs: l, rhs: r } => {
            // `x ?? y`: `x` is a heap-boxed Option (null = None). If present,
            // dereference the payload (typed by the destination); else use `y`.
            if matches!(op, BinOp::NullCoalesce) {
                let lop = emit_operand(l);
                let rop = emit_operand(r);
                writeln!(out, "    {lhs} = (({lop}) != 0) ? *(__typeof__({lhs})*)({lop}) : ({rop});").unwrap();
            } else {
                writeln!(out, "    {} = {};", lhs, emit_binop(op, l, r, locals)).unwrap();
            }
        }
        Rvalue::UnOp { op, arg } => {
            let sym = match op { UnOp::Neg => "-", UnOp::Not => "!" };
            writeln!(out, "    {} = {}({});", lhs, sym, emit_operand(arg)).unwrap();
        }
        Rvalue::Call { func, args } => {
            let args_str = emit_call_args(func, args);
            writeln!(out, "    {} = {}({});", lhs, emit_callee(func, locals), args_str).unwrap();
        }
        Rvalue::Field { base, field } => {
            // For an all-nullary enum (represented as a plain int), the value *is*
            // its tag — there is no `.tag` field to read.
            if field == "tag" {
                if let Operand::Local(id) = base {
                    if let Some(Ty::Named { name, .. }) = locals.iter().find(|l| l.id == *id).map(|l| &l.ty) {
                        if nullary_enums.contains(name) {
                            writeln!(out, "    {} = {};", lhs, emit_operand(base)).unwrap();
                            return;
                        }
                    }
                }
            }
            // A numeric field is a tuple index. Tuples are heap lists, so read the
            // element by index and cast it back to the destination's C type (the
            // element was stored as `(void*)(intptr_t)value` by the Tuple aggregate).
            if let Ok(idx) = field.parse::<usize>() {
                let dest_ty = locals.iter().find(|l| l.id == *dest)
                    .map(|l| l.ty.clone()).unwrap_or(Ty::Error);
                let slot = format!("certo_list_get_or_panic((CertoList*)({}), {})", emit_operand(base), idx);
                writeln!(out, "    {} = {};", lhs, unbox_value(&slot, &dest_ty)).unwrap();
            } else {
                writeln!(out, "    {} = {}.{};", lhs, emit_operand(base), field).unwrap();
            }
        }
        Rvalue::Aggregate(kind, ops) => {
            match kind {
                AggregateKind::Tuple => {
                    // Represent tuples as CertoList* — same as arrays. Each element
                    // is boxed with its own type so e.g. a Float keeps its bits.
                    if ops.is_empty() {
                        writeln!(out, "    {} = (void*)0;", lhs).unwrap();
                    } else {
                        let elems = ops.iter()
                            .map(|o| box_value(&emit_operand(o), &operand_ty(o, locals)))
                            .collect::<Vec<_>>().join(", ");
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
                        let elems = ops.iter()
                            .map(|o| box_value(&emit_operand(o), &operand_ty(o, locals)))
                            .collect::<Vec<_>>().join(", ");
                        writeln!(out, "    {} = certo_list_of({}, {});", lhs, ops.len(), elems).unwrap();
                    }
                }
            }
        }
        Rvalue::Spawn { args, .. } => {
            // Heap-allocate the context, copy the (already-evaluated) arguments in,
            // and launch the worker on a new thread. The handle is the context ptr.
            let ctx    = format!("__certo_ctx_{}_{}_t", fn_cname, *spawn_idx);
            let worker = format!("__certo_worker_{}_{}", fn_cname, *spawn_idx);
            *spawn_idx += 1;
            writeln!(out, "    {{").unwrap();
            writeln!(out, "      {ctx}* _sc = ({ctx}*)malloc(sizeof({ctx}));").unwrap();
            for (i, a) in args.iter().enumerate() {
                writeln!(out, "      _sc->a{} = {};", i, emit_operand(a)).unwrap();
            }
            writeln!(out, "      _sc->hdr.thread = __certo_thread_spawn({worker}, _sc);").unwrap();
            writeln!(out, "      {} = (void*)_sc;", lhs).unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Rvalue::Join { task, ret_ty } => {
            // Wait for the thread, read the result through a layout-compatible
            // accessor, then free the context.
            let t = emit_operand(task);
            let rmangle = mangle(ret_ty);
            writeln!(out, "    __certo_thread_join(((__certo_task_hdr_t*)({t}))->thread);").unwrap();
            writeln!(out, "    {} = ((__certo_join_{}_t*)({t}))->result;", lhs, rmangle).unwrap();
            writeln!(out, "    free((void*)({t}));").unwrap();
        }
        Rvalue::BoxSome { value, ty } => {
            // Heap-box the payload with its own C type so its bits are preserved
            // (crucial for `double`, and so `Some(0)` differs from `None`).
            let cty = ty_to_c(ty);
            writeln!(out, "    {{").unwrap();
            writeln!(out, "      {cty}* _ob = ({cty}*)malloc(sizeof({cty}));").unwrap();
            writeln!(out, "      *_ob = ({cty})({});", emit_operand(value)).unwrap();
            writeln!(out, "      {} = (void*)_ob;", lhs).unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Rvalue::UnboxSome { opt, ty } => {
            let cty = ty_to_c(ty);
            writeln!(out, "    {} = *({cty}*)({});", lhs, emit_operand(opt)).unwrap();
        }
        Rvalue::Box { value, ty } => {
            writeln!(out, "    {} = {};", lhs, box_value(&emit_operand(value), ty)).unwrap();
        }
        Rvalue::Unbox { value, ty } => {
            writeln!(out, "    {} = {};", lhs, unbox_value(&emit_operand(value), ty)).unwrap();
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
            let args_str = emit_call_args(func, args);
            writeln!(out, "    {} = {}({});", local_name(*dest), emit_callee(func, locals), args_str).unwrap();
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

/// Emit the callee of a call expression. A named function (`Operand::Global`)
/// already has its real, correctly-typed C declaration, so it's called
/// directly. A function *value* held in a local, though, is statically typed
/// as the opaque zero-arg `certo_fn_t` (see `ty_to_c`) — calling it without a
/// cast would use that 0-arg signature regardless of how many arguments the
/// call actually passes. Cast it to the real signature first, using the
/// local's own `Ty::Fn { params, ret }`.
fn emit_callee(func: &Operand, locals: &[MirLocalDecl]) -> String {
    let Operand::Local(id) = func else { return emit_operand(func) };
    let Some(Ty::Fn { params, ret }) = locals.iter().find(|l| l.id == *id).map(|l| &l.ty) else {
        return emit_operand(func);
    };
    let param_str = if params.is_empty() {
        "void".to_string()
    } else {
        params.iter().map(ty_to_c).collect::<Vec<_>>().join(", ")
    };
    format!("(({}(*)({}))({}))", ret_ty_to_c(ret), param_str, emit_operand(func))
}

fn emit_call_args(func: &Operand, args: &[Operand]) -> String {
    let func_c = emit_operand(func);
    args.iter()
        .enumerate()
        .map(|(idx, arg)| emit_call_arg(&func_c, idx, arg))
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_call_arg(func_c: &str, idx: usize, arg: &Operand) -> String {
    let expr = emit_operand(arg);
    match (func_c, idx) {
        // List<T> stores generic items in a void* slot. Text lowers to
        // certo_text_t (const char*), so make the intentional ABI cast explicit.
        ("certo_list_push", 1) => format!("(void*)({expr})"),
        _ => expr,
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

/// Coerce a concat operand to `certo_text_t`. F-string interpolations desugar to
/// a `Concat` chain (`lower.rs`), but the interpolated value may be any type;
/// `certo_text_concat` expects a string, so a non-Text operand passed verbatim is
/// read as a pointer and segfaults. Wrap such operands in the matching runtime
/// `*_to_text` conversion. Genuine `++` is unaffected: typeck forces both sides to
/// Text, so only f-string operands ever reach here non-Text.
fn coerce_to_text(op: &Operand, expr: String, locals: &[MirLocalDecl]) -> String {
    if operand_is_text(op, locals) {
        return expr;
    }
    let ty = match op {
        Operand::Local(id) => locals.iter().find(|l| l.id == *id).map(|l| l.ty.clone()),
        Operand::Const(MirConst::Int(_))     => Some(Ty::Int),
        Operand::Const(MirConst::Float(_))   => Some(Ty::Float),
        Operand::Const(MirConst::Bool(_))    => Some(Ty::Bool),
        Operand::Const(MirConst::Decimal(_)) => Some(Ty::Decimal),
        _ => None,
    };
    match ty {
        Some(Ty::Int)     => format!("certo_int_to_text({})", expr),
        Some(Ty::Float)   => format!("certo_float_to_text({})", expr),
        Some(Ty::Bool)    => format!("certo_bool_to_text({})", expr),
        Some(Ty::Decimal) => format!("certo_decimal_to_text({})", expr),
        _ => expr,
    }
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
        BinOp::NullCoalesce => format!("certo_coalesce((void*)({lhs}), (void*)({rhs}))"),
        BinOp::Concat       => format!("certo_text_concat({}, {})", coerce_to_text(l, lhs, locals), coerce_to_text(r, rhs, locals)),
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
