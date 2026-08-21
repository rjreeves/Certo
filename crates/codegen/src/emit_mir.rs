use std::collections::{HashSet, HashMap};
use std::fmt::Write as FmtWrite;
use certo_mir::{MirFn, MirStmt, MirLocalDecl, Rvalue, Operand, MirConst, Terminator, AggregateKind};
use certo_hir::{BinOp, UnOp};
use certo_typeck::Ty;
use crate::ty_to_c::{ty_to_c, ret_ty_to_c, mangle, c_ident};

// ------------------------------------------------------------------ //
// Source-mapped `#line` directives for `certo test --coverage` (BACKLOG
// item 126). Opt-in only (`Option<&LineMap>` threaded through emission) —
// a normal `certo build`/`certo run` never constructs one, so ordinary
// compile errors keep pointing at the *generated* C, matching every
// existing user's expectations. Only `certo test --coverage`'s own C
// generation (`crates/testrunner/src/harness.rs`) opts in.
//
// NOTE: `llvm-cov` itself completely ignores `#line` for coverage
// attribution (confirmed by direct testing against LLVM 22 — neither
// `llvm-cov report`'s file grouping nor `llvm-cov show`'s line numbers
// shift for a `#line` pragma). These directives exist so
// `crates/testrunner/src/coverage.rs` can re-derive the same mapping
// itself, by scanning the exact compiled `c_src` text, and remap
// `llvm-cov export`'s generated-C-line hit data back to `.cto` lines on
// our own side. See that module's doc comment for the full picture.
pub struct LineMap<'a> {
    pub filename: &'a str,
    /// `filename` with `\` and `"` escaped for embedding in a C string
    /// literal — required on Windows, where paths contain `\` that the C
    /// preprocessor would otherwise read as escape sequences (e.g. `\U`,
    /// `\A`) inside the `#line "<file>"` directive.
    filename_escaped: String,
    /// `line_starts[i]` = byte offset of the first byte of line `i+1`
    /// (1-based lines, so `line_starts[0]` is always `0`).
    line_starts: Vec<u32>,
}

impl<'a> LineMap<'a> {
    pub fn new(filename: &'a str, source: &str) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' { line_starts.push((i + 1) as u32); }
        }
        let filename_escaped = filename.replace('\\', "\\\\").replace('"', "\\\"");
        LineMap { filename, filename_escaped, line_starts }
    }

    /// 1-based line number containing `byte_offset`.
    pub fn line_of(&self, byte_offset: u32) -> u32 {
        match self.line_starts.binary_search(&byte_offset) {
            Ok(i) => (i + 1) as u32,
            Err(i) => i.max(1) as u32,
        }
    }
}

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
    /// BACKLOG item 186 — true when `func_c` is a dedicated function
    /// synthesized just for this call site (`lift_spawn_body`), which
    /// therefore has a hidden trailing cancel-token parameter the worker
    /// passes its own header pointer through. False for a direct call to an
    /// existing, possibly-shared named function (`spawn f(a, b)`), whose
    /// signature must not be touched.
    pub is_lifted:   bool,
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
            if let MirStmt::Assign { rvalue: Rvalue::Spawn { func, args, ret_ty, is_lifted }, .. } = stmt {
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
                    is_lifted: *is_lifted,
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
        // A lifted worker (BACKLOG item 186) gets one extra trailing
        // argument — its own header pointer, for its loops' cancellation
        // checkpoints — appended here rather than stored in the context
        // struct like a real capture, since it's this struct's own address,
        // not a value known at spawn time. An ordinary shared function
        // (`is_lifted` false) gets no such argument — its signature and
        // every other call site of it are untouched.
        let mut call_args: Vec<String> = (0..s.arg_tys.len()).map(|i| format!("c->a{}", i)).collect();
        if s.is_lifted {
            call_args.push("&c->hdr".to_string());
        }
        writeln!(out, "static void* {}(void* _p) {{", s.worker_name).unwrap();
        writeln!(out, "    {}* c = ({}*)_p;", s.ctx_name, s.ctx_name).unwrap();
        writeln!(out, "    c->result = {}({});", s.func_c, call_args.join(", ")).unwrap();
        // Ordinary spawn/parallel joins never call __certo_task_hdr_try_abandon,
        // so this CAS always wins for them (behaviorally identical to the old
        // unconditional signal-done) — the abandon branch below only ever runs
        // for a task a `withTimeout(d) { ... }` (BACKLOG item 122) gave up
        // waiting on, where the worker itself must own cleanup instead of the
        // joiner, since nothing else will ever join or free this context.
        writeln!(out, "    if (__certo_task_hdr_try_finish(&c->hdr)) {{").unwrap();
        writeln!(out, "        __certo_task_signal_done(&c->hdr);").unwrap();
        writeln!(out, "    }} else {{").unwrap();
        writeln!(out, "        __certo_task_hdr_abandon_cleanup(&c->hdr);").unwrap();
        writeln!(out, "        free(c);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "    return 0;").unwrap();
        writeln!(out, "}}").unwrap();
    }
}

/// `panic(msg)`/`unreachable()`/`todo()` are all `∀a. (...) -> a` at the
/// Certo level (usable in any expression position) but their real C
/// implementations are genuinely `noreturn void` (`certo_panic`, and the
/// `certo_unreachable()`/`certo_todo()` macros that expand to it) — unlike
/// an ordinary `Unit`-returning Certo function, which fakes a capturable
/// `int64_t` zero return so call sites can always assign the result
/// uniformly. Assigning a `void` call's result is a C compile error, so a
/// call to any of these three must be emitted as a bare statement instead.
/// `unreachable`/`todo` were excluded here until BACKLOG item 210 fixed
/// their own separate, unrelated arity bug (typeck required a `Text` arg
/// that their C macros don't accept) — with that fixed, they now reach
/// this codegen path too and need the identical treatment `panic` already
/// gets, or they'd hit this exact "assigning to 'int64_t' from incompatible
/// type 'void'" error instead.
fn is_void_noreturn_callee(func: &Operand) -> bool {
    matches!(func, Operand::Global(name) if matches!(name.as_str(), "panic" | "unreachable" | "todo"))
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
    if ty.needs_heap_box() {
        let cty = ty_to_c(ty);
        // GNU statement-expression — clang/gcc are already a hard
        // requirement for this whole toolchain (BACKLOG item 104: MSVC's
        // cl.exe isn't a supported C compiler), and box_value must stay a
        // single composable expression: it's used inline as a function-call
        // argument (`certo_list_of(n, box_value(...), ...)`), not only as a
        // standalone assignment RHS, so a plain malloc+copy+return can't be
        // phrased as one portable C99 expression when `value` might be an
        // rvalue (a call result), not an addressable variable.
        format!("({{ {cty}* _cb = ({cty}*)malloc(sizeof({cty})); *_cb = ({value}); (void*)_cb; }})")
    } else {
        match ty {
            // A double must be bit-cast, never numeric-converted.
            Ty::Float => format!("(void*)__certo_f2i({})", value),
            // Everything else here is already pointer-sized (ints, bool, Text,
            // and handle types like Option/List/Result/Map/Tuple/nullary-enum).
            _ => format!("(void*)(intptr_t)({})", value),
        }
    }
}

/// Recover a value of type `ty` from a pointer-sized slot.
fn unbox_value(slot: &str, ty: &Ty) -> String {
    if ty.needs_heap_box() {
        let cty = ty_to_c(ty);
        format!("(*({cty}*)({slot}))")
    } else {
        match ty {
            Ty::Float => format!("__certo_i2f((int64_t)(intptr_t)({}))", slot),
            _ => format!("({})(intptr_t)({})", ty_to_c(ty), slot),
        }
    }
}

/// Emit a single MIR function as a C function definition with an optional prefix
/// (e.g. `"CERTO_EXPORT "` for shared library builds). `line_map` is `Some`
/// only for `certo test --coverage` builds (BACKLOG item 126) — see the
/// `LineMap` doc comment above.
pub fn emit_fn_with_prefix(f: &MirFn, prefix: &str, nullary_enums: &HashSet<String>, eq_types: &HashSet<String>, variant_parent: &HashMap<String, String>, line_map: Option<&LineMap>, out: &mut String) {
    // Temporarily intercept the signature line to inject the prefix.
    let mut body = String::new();
    emit_fn_inner(f, prefix, nullary_enums, eq_types, variant_parent, line_map, &mut body);
    out.push_str(&body);
}

fn emit_fn_inner(f: &MirFn, prefix: &str, nullary_enums: &HashSet<String>, eq_types: &HashSet<String>, variant_parent: &HashMap<String, String>, line_map: Option<&LineMap>, out: &mut String) {
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
    // Tracks the last line a `#line` directive was emitted for, so an
    // unbroken run of statements from the same source line doesn't repeat
    // one before every single generated C statement.
    let mut last_line: Option<u32> = None;
    for bb in &f.blocks {
        writeln!(out, "  bb{}:", bb.id).unwrap();
        for stmt in &bb.stmts {
            if let Some(lm) = line_map {
                let MirStmt::Assign { span, .. } = stmt;
                if span.start != 0 || span.end != 0 {
                    let line = lm.line_of(span.start);
                    if last_line != Some(line) {
                        writeln!(out, "#line {} \"{}\"", line, lm.filename_escaped).unwrap();
                        last_line = Some(line);
                    }
                } else if last_line.is_some() {
                    // Synthetic statement with no source correspondence
                    // (e.g. an injected boxing/defer shim) — see the
                    // end-of-function reset above for why this matters.
                    writeln!(out, "#line 1 \"<generated>\"").unwrap();
                    last_line = None;
                }
            }
            emit_stmt(stmt, &f.locals, &fn_cname, &mut spawn_idx, nullary_enums, eq_types, variant_parent, out);
        }
        if let Some(term) = &bb.terminator {
            emit_terminator(term, ret_ty, &f.locals, out);
        }
    }

    // `#line` attribution otherwise persists (per real C semantics) into
    // whatever gets emitted next — the next function's local decls, its own
    // unspanned preamble, or (worst case) the rest of the file — silently
    // misattributing it to this function's last source line. Reset back to
    // an untracked state now that this function is done. `coverage.rs`'s
    // `build_line_remap` recognizes the `<generated>` sentinel filename and
    // treats it as "no `.cto` mapping" rather than a real target file.
    if line_map.is_some() && last_line.is_some() {
        writeln!(out, "#line 1 \"<generated>\"").unwrap();
    }
    writeln!(out, "}}").unwrap();
}

fn emit_stmt(stmt: &MirStmt, locals: &[MirLocalDecl], fn_cname: &str, spawn_idx: &mut u32, nullary_enums: &HashSet<String>, eq_types: &HashSet<String>, variant_parent: &HashMap<String, String>, out: &mut String) {
    let MirStmt::Assign { dest, rvalue, .. } = stmt;
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
                writeln!(out, "    {} = {};", lhs, emit_binop(op, l, r, locals, eq_types, variant_parent)).unwrap();
            }
        }
        Rvalue::UnOp { op, arg } => {
            let sym = match op { UnOp::Neg => "-", UnOp::Not => "!" };
            writeln!(out, "    {} = {}({});", lhs, sym, emit_operand(arg)).unwrap();
        }
        Rvalue::Call { func, args } => {
            let args_str = emit_call_args(func, args, locals);
            if is_void_noreturn_callee(func) {
                // `panic(msg)` — genuinely `noreturn void` in C (unlike an
                // ordinary `Unit`-returning Certo function, which fakes a
                // capturable `int64_t` return of 0); assigning its result
                // is a compile error. `lhs` is left unset, which is sound
                // since `noreturn` means nothing after this line executes.
                writeln!(out, "    {}({});", emit_callee(func, locals), args_str).unwrap();
            } else {
                writeln!(out, "    {} = {}({});", lhs, emit_callee(func, locals), args_str).unwrap();
            }
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
            writeln!(out, "      __certo_task_hdr_init(&_sc->hdr);").unwrap();
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
        Rvalue::JoinTimed { task, deadline, ret_ty } => {
            // `parallel(timeout: ...) { ... }` — BACKLOG item 81. `deadline`
            // is an absolute monotonic-clock millisecond value shared by
            // every task in the block (computed once in HIR); the remaining
            // budget for *this* join is whatever's left of it right now, so
            // time already spent waiting on earlier tasks is correctly
            // deducted rather than each task getting its own full timeout.
            let t = emit_operand(task);
            let d = emit_operand(deadline);
            let rmangle = mangle(ret_ty);
            writeln!(out, "    {{").unwrap();
            writeln!(out, "      int64_t _remaining_ms = ({d}) - certo_monotonic_millis();").unwrap();
            writeln!(out, "      if (!__certo_thread_join_timed((__certo_task_hdr_t*)({t}), _remaining_ms)) {{").unwrap();
            writeln!(out, "        certo_panic(\"parallel(timeout: ...) block exceeded its timeout\");").unwrap();
            writeln!(out, "      }}").unwrap();
            writeln!(out, "      {} = ((__certo_join_{}_t*)({t}))->result;", lhs, rmangle).unwrap();
            writeln!(out, "      free((void*)({t}));").unwrap();
            writeln!(out, "    }}").unwrap();
        }
        Rvalue::JoinTimedCancel { task, deadline, ret_ty } => {
            // `withTimeout(d) { ... }` — BACKLOG item 122. Unlike `JoinTimed`
            // above, never panics: on timeout the task is safely *abandoned*
            // instead of leaked (via the atomic ownership handoff in
            // `__certo_task_hdr_try_abandon`/`__certo_task_hdr_try_finish`,
            // `crates/codegen/src/emit_module.rs`) and this yields a null
            // (`None`) `Option` pointer rather than a boxed value. A task
            // that finished in the tiny race window between the timed wait
            // failing and the abandon attempt is treated as an on-time
            // success (`Some`), not discarded — `__certo_thread_join` there
            // returns near-instantly since the worker is already done or
            // finishing.
            let t = emit_operand(task);
            let d = emit_operand(deadline);
            let cty = ty_to_c(ret_ty);
            let rmangle = mangle(ret_ty);
            writeln!(out, "    {{").unwrap();
            writeln!(out, "      __certo_task_hdr_t* _wt_hdr = (__certo_task_hdr_t*)({t});").unwrap();
            writeln!(out, "      int64_t _remaining_ms = ({d}) - certo_monotonic_millis();").unwrap();
            writeln!(out, "      bool _wt_ok;").unwrap();
            writeln!(out, "      if (__certo_thread_join_timed(_wt_hdr, _remaining_ms)) {{").unwrap();
            writeln!(out, "        _wt_ok = true;").unwrap();
            writeln!(out, "      }} else if (!__certo_task_hdr_try_abandon(_wt_hdr)) {{").unwrap();
            writeln!(out, "        __certo_thread_join(_wt_hdr->thread);").unwrap();
            writeln!(out, "        _wt_ok = true;").unwrap();
            writeln!(out, "      }} else {{").unwrap();
            writeln!(out, "        _wt_ok = false;").unwrap();
            writeln!(out, "      }}").unwrap();
            writeln!(out, "      if (_wt_ok) {{").unwrap();
            writeln!(out, "        {cty}* _ob = ({cty}*)malloc(sizeof({cty}));").unwrap();
            writeln!(out, "        *_ob = ((__certo_join_{}_t*)({t}))->result;", rmangle).unwrap();
            writeln!(out, "        free((void*)({t}));").unwrap();
            writeln!(out, "        {} = (void*)_ob;", lhs).unwrap();
            writeln!(out, "      }} else {{").unwrap();
            writeln!(out, "        {} = NULL;", lhs).unwrap();
            writeln!(out, "      }}").unwrap();
            writeln!(out, "    }}").unwrap();
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
        Rvalue::UnwrapOptStructBox { value, ty: _ } => {
            let v = emit_operand(value);
            writeln!(out, "    {} = ({v}) ? *(void**)({v}) : NULL;", lhs).unwrap();
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
            // See `Rvalue::Call`'s identical special case: `panic(msg)`'s C
            // implementation is genuinely `noreturn void`.
            if is_void_noreturn_callee(func) {
                writeln!(out, "    {}({});", emit_callee(func, locals), args_str).unwrap();
            } else {
                writeln!(out, "    {} = {}({});", local_name(*dest), emit_callee(func, locals), args_str).unwrap();
            }
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
/// as the 2-word closure struct `certo_fn_t { void* fn; void* env; }` (see
/// `ty_to_c` — BACKLOG item 140) — its `.fn` slot is the real callee, cast
/// to the real signature with a leading `void* env` parameter (every
/// generated lambda function uniformly takes its closure environment as its
/// own first parameter, whether or not it actually captures anything, so
/// caller and callee always agree on arity regardless of which specific
/// lambda ends up there). See `emit_call_args` for the paired `.env` first
/// argument this cast's signature expects.
fn emit_callee(func: &Operand, locals: &[MirLocalDecl]) -> String {
    let Operand::Local(id) = func else { return emit_operand(func) };
    let Some(Ty::Fn { params, ret }) = locals.iter().find(|l| l.id == *id).map(|l| &l.ty) else {
        return emit_operand(func);
    };
    let mut param_strs: Vec<String> = vec!["void*".to_string()];
    param_strs.extend(params.iter().map(ty_to_c));
    format!("(({}(*)({}))(({}).fn))", ret_ty_to_c(ret), param_strs.join(", "), emit_operand(func))
}

/// The closure environment argument a call through a local `Ty::Fn` value
/// must pass first — `None` for a direct call to a named global function,
/// which has no closure struct at all (BACKLOG item 140; see `emit_callee`).
fn emit_call_env_arg(func: &Operand, locals: &[MirLocalDecl]) -> Option<String> {
    let Operand::Local(id) = func else { return None };
    if !locals.iter().any(|l| l.id == *id && matches!(l.ty, Ty::Fn { .. })) { return None; }
    Some(format!("({}).env", emit_operand(func)))
}

fn emit_call_args(func: &Operand, args: &[Operand], locals: &[MirLocalDecl]) -> String {
    let func_c = emit_operand(func);
    let mut parts: Vec<String> = emit_call_env_arg(func, locals).into_iter().collect();
    parts.extend(args.iter().enumerate().map(|(idx, arg)| emit_call_arg(&func_c, idx, arg, locals)));
    parts.join(", ")
}

fn emit_call_arg(func_c: &str, idx: usize, arg: &Operand, locals: &[MirLocalDecl]) -> String {
    let expr = emit_operand(arg);
    match (func_c, idx) {
        // List<T> stores generic items in a void* slot — for most types a
        // plain cast is fine, but a struct-typed item (BACKLOG item 134)
        // needs the same real heap-boxing `box_value` uses for List/Tuple
        // literal elements, not a cast that won't even compile for a struct.
        // `List.upsert` (BACKLOG item 209) takes its new item at the same
        // argument position as `List.push` and needs the identical
        // treatment, for the identical reason.
        ("certo_list_push", 1) | ("certo_list_upsert", 1) => box_value(&expr, &operand_ty(arg, locals)),
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
        Operand::Const(MirConst::Decimal(_)) => Some(Ty::Decimal(None)),
        _ => None,
    };
    match ty {
        Some(Ty::Int)        => format!("certo_int_to_text({})", expr),
        Some(Ty::Float)      => format!("certo_float_to_text({})", expr),
        Some(Ty::Bool)       => format!("certo_bool_to_text({})", expr),
        Some(Ty::Decimal(_)) => format!("certo_decimal_to_text({})", expr),
        _ => expr,
    }
}

/// The Certo-level type *name* of an operand, when it's a plain named type
/// (record or enum) — used only to decide whether `emit_eq` should call a
/// generated `certo_eq_<Name>` function. Handles one case `operand_ty`
/// itself can't: a bare nullary-variant *constant* of a mixed (some
/// variants carry a payload, some don't) sum type is `Operand::Global`,
/// not `Operand::Local`, so it has no `MirLocalDecl` to look its type up
/// from at all — confirmed by direct testing (`Origin == Origin`, where
/// `Origin` is a payload-free variant of a `Shape` that also has
/// `Circle(radius: Float)`, failed to compile: `certo_origin`'s own
/// static type is the *full* `Shape` struct, since a mixed enum's values
/// are never the plain-int representation an *all*-nullary enum gets, but
/// nothing recovered that). `variant_parent` (built in
/// `crates/codegen/src/emit_module.rs` from the same sum-type declarations
/// `eq_types` itself is derived from) closes that gap.
fn operand_type_name(op: &Operand, locals: &[MirLocalDecl], variant_parent: &HashMap<String, String>) -> Option<String> {
    if let Ty::Named { name, args } = operand_ty(op, locals) {
        if args.is_empty() { return Some(name); }
    }
    if let Operand::Global(name) = op {
        if let Some(parent) = variant_parent.get(name) {
            return Some(parent.clone());
        }
    }
    None
}

/// The real equality-comparison C expression for two operands of the same
/// type, given what's known about it — BACKLOG item 201. `Text` was
/// already handled this way (`certo_text_eq`); `Decimal`/`UUID` are both
/// plain-value C structs that don't compile with a bare `==` either
/// (confirmed: `d1 == d2` alone fails to compile, a separate pre-existing
/// gap this closes too, found while scoping this item), and a
/// user record/payload-enum gets its own generated `certo_eq_<Name>`
/// (`eq_types`, built in `crates/codegen/src/emit_module.rs`) — anything
/// else (scalars, nullary enums, and anything `eq_types` doesn't know
/// about — List/Option/Map/Tuple/Fn-typed values, deliberately out of
/// this item's scope, see item 200's own design sketch) falls back to the
/// original bare `==`, unchanged.
fn emit_eq(
    l: &Operand, r: &Operand, locals: &[MirLocalDecl],
    eq_types: &HashSet<String>, variant_parent: &HashMap<String, String>,
    lhs: &str, rhs: &str,
) -> String {
    let ty = match operand_ty(l, locals) {
        Ty::Error => operand_ty(r, locals),
        other => other,
    };
    match &ty {
        Ty::Text => return format!("certo_text_eq({lhs}, {rhs})"),
        Ty::Decimal(_) => return format!("certo_decimal_eq({lhs}, {rhs})"),
        Ty::Uuid => return format!("certo_uuid_eq({lhs}, {rhs})"),
        _ => {}
    }
    let name = operand_type_name(l, locals, variant_parent)
        .or_else(|| operand_type_name(r, locals, variant_parent));
    if let Some(name) = name {
        if eq_types.contains(&name) {
            return format!("certo_eq_{}({lhs}, {rhs})", c_ident(&name));
        }
    }
    format!("({lhs} == {rhs})")
}

fn emit_binop(
    op: &BinOp, l: &Operand, r: &Operand, locals: &[MirLocalDecl],
    eq_types: &HashSet<String>, variant_parent: &HashMap<String, String>,
) -> String {
    let lhs = emit_operand(l);
    let rhs = emit_operand(r);
    match op {
        BinOp::Add  => format!("({} + {})", lhs, rhs),
        BinOp::Sub  => format!("({} - {})", lhs, rhs),
        BinOp::Mul  => format!("({} * {})", lhs, rhs),
        BinOp::Div  => format!("({} / {})", lhs, rhs),
        BinOp::Rem  => format!("({} % {})", lhs, rhs),
        BinOp::Pow  => format!("certo_pow({}, {})", lhs, rhs),
        BinOp::Eq   => emit_eq(l, r, locals, eq_types, variant_parent, &lhs, &rhs),
        BinOp::NotEq => format!("(!{})", emit_eq(l, r, locals, eq_types, variant_parent, &lhs, &rhs)),
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

#[cfg(test)]
mod line_map_tests {
    use super::LineMap;

    #[test]
    fn line_of_finds_correct_1_based_line() {
        let src = "line1\nline2\nline3\n";
        let lm = LineMap::new("f.cto", src);
        assert_eq!(lm.line_of(0), 1);  // 'l' of line1
        assert_eq!(lm.line_of(6), 2);  // 'l' of line2
        assert_eq!(lm.line_of(12), 3); // 'l' of line3
    }

    #[test]
    fn escapes_windows_path_for_c_string_literal() {
        // Without escaping, clang reads `\U`/`\A`/etc as invalid escape
        // sequences inside the `#line "<file>"` directive — confirmed by
        // direct repro compiling a Windows temp path unescaped.
        let lm = LineMap::new("C:\\Users\\bob\\a\"b.cto", "");
        assert_eq!(lm.filename_escaped, "C:\\\\Users\\\\bob\\\\a\\\"b.cto");
    }
}
