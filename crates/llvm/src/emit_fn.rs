//! Emit a single Certo function as LLVM IR text.
//!
//! # Strategy
//!
//! ## Type information
//! HIR lowering leaves all types as `Ty::Error` (placeholders for the type
//! checker).  We recover correct types by:
//!  1. Reading declared param/ret types from the AST `FnDecl`.
//!  2. Propagating types through MIR assignments (BinOp → result type,
//!     Use → same type as operand, constants → known types).
//!  3. Falling back to `ptr` for anything unresolvable (runtime calls, etc.).
//!
//! ## alloca/store/load pattern
//! Every local gets an `alloca` in the entry block.  Assignments become
//! `store`s and reads become `load`s.  LLVM's `mem2reg` pass (invoked by
//! `clang -O1` or `opt -O1`) converts these to proper SSA φ-nodes.

use std::collections::HashMap;

use certo_ast::decl::FnDecl;
use certo_hir::{BinOp, UnOp};
use certo_mir::{MirFn, MirLocal, MirStmt, Rvalue, Operand, MirConst, Terminator, SwitchTarget};

use crate::ctx::ModuleCtx;
use crate::ty::{ast_ty_to_llvm, ast_ret_ty_to_llvm, llvm_fn_name};

// ------------------------------------------------------------------ //
// Public entry point
// ------------------------------------------------------------------ //

/// Emit LLVM IR for one function.
///
/// Takes the original AST `FnDecl` (for declared types) alongside the `MirFn`
/// (for the control-flow graph).  Side-effects `ctx` with string literals and
/// external declarations.
pub fn emit_fn(ast_fn: &FnDecl, mir_fn: &MirFn, ctx: &mut ModuleCtx) -> String {
    FnEmitter::new(ast_fn, mir_fn, ctx).emit()
}

// ------------------------------------------------------------------ //
// Type inference
// ------------------------------------------------------------------ //

/// Infer LLVM type strings for every MIR local by:
///  1. Seeding from AST param/ret declarations.
///  2. Propagating through assignments until stable.
fn compute_local_types(ast_fn: &FnDecl, mir_fn: &MirFn) -> HashMap<MirLocal, String> {
    let mut map: HashMap<MirLocal, String> = HashMap::new();

    // Ret slot (local 0) ← declared return type
    let ret_llty = ast_ret_ty_to_llvm(
        ast_fn.ret_ty.as_ref().map(|s| &s.node)
    );
    // For the alloca, Unit becomes i8; the actual `ret void` is handled elsewhere.
    map.insert(0, if ret_llty == "void" { "i8".into() } else { ret_llty });

    // Params: match by name
    for ast_param in &ast_fn.params {
        let llty = ast_ty_to_llvm(&ast_param.ty.node);
        if let Some(ml) = mir_fn.locals.iter().find(|l| l.name == ast_param.name.node) {
            map.insert(ml.id, llty);
        }
    }

    // Propagation: repeat until no new types are learned
    let mut changed = true;
    while changed {
        changed = false;
        for bb in &mir_fn.blocks {
            for stmt in &bb.stmts {
                let MirStmt::Assign { dest, rvalue } = stmt;
                if map.contains_key(dest) { continue; }
                if let Some(t) = infer_rvalue(&map, rvalue) {
                    map.insert(*dest, t);
                    changed = true;
                }
            }
            // Call terminator destinations default to ptr
            if let Some(Terminator::Call { dest, .. }) = &bb.terminator {
                if !map.contains_key(dest) {
                    map.insert(*dest, "ptr".into());
                    changed = true;
                }
            }
        }
    }

    // Fill any remaining locals with ptr
    for local in &mir_fn.locals {
        map.entry(local.id).or_insert_with(|| "ptr".into());
    }

    map
}

fn infer_rvalue(map: &HashMap<MirLocal, String>, rv: &Rvalue) -> Option<String> {
    match rv {
        Rvalue::Use(op)          => operand_llty(map, op),
        Rvalue::UnOp { arg, .. } => operand_llty(map, arg),
        Rvalue::BinOp { op, lhs, .. } => {
            let lty = operand_llty(map, lhs)?;
            Some(binop_result_llty(op, &lty))
        }
        Rvalue::Call { .. }      => Some("ptr".into()),
        Rvalue::Field { .. }     => Some("ptr".into()),
        Rvalue::Aggregate(_, _)  => Some("ptr".into()),
        // Concurrency is not supported by the experimental LLVM backend.
        Rvalue::Spawn { .. }     => Some("ptr".into()),
        Rvalue::Join { .. }      => Some("ptr".into()),
        Rvalue::BoxSome { .. }   => Some("ptr".into()),
        Rvalue::UnboxSome { .. } => Some("ptr".into()),
    }
}

fn operand_llty(map: &HashMap<MirLocal, String>, op: &Operand) -> Option<String> {
    match op {
        Operand::Local(id)  => map.get(id).cloned(),
        Operand::Const(c)   => Some(const_llty(c)),
        Operand::Global(_)  => Some("ptr".into()),
    }
}

fn const_llty(c: &MirConst) -> String {
    match c {
        MirConst::Int(_)                            => "i64".into(),
        MirConst::Float(_)                          => "double".into(),
        MirConst::Bool(_)                           => "i1".into(),
        MirConst::Unit                              => "i8".into(),
        MirConst::Str(_) | MirConst::Decimal(_)
                          | MirConst::Uuid(_)       => "ptr".into(),
    }
}

fn binop_result_llty(op: &BinOp, lty: &str) -> String {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul |
        BinOp::Div | BinOp::Rem | BinOp::Pow      => lty.into(),
        BinOp::Eq  | BinOp::NotEq | BinOp::Lt     |
        BinOp::LtEq | BinOp::Gt  | BinOp::GtEq   |
        BinOp::And | BinOp::Or                     => "i1".into(),
        BinOp::NullCoalesce                         => "ptr".into(),
        BinOp::Concat                               => "ptr".into(),
    }
}

// ------------------------------------------------------------------ //
// Per-function emitter
// ------------------------------------------------------------------ //

struct FnEmitter<'ctx> {
    ast_fn:    FnDecl,           // owned clone
    mir_fn:    MirFn,
    ctx:       &'ctx mut ModuleCtx,
    local_ty:  HashMap<MirLocal, String>,  // LLVM type for each local
    out:       String,
    tmp:       u32,
}

impl<'ctx> FnEmitter<'ctx> {
    fn new(ast_fn: &FnDecl, mir_fn: &MirFn, ctx: &'ctx mut ModuleCtx) -> Self {
        let local_ty = compute_local_types(ast_fn, mir_fn);
        Self {
            ast_fn: ast_fn.clone(),
            mir_fn: mir_fn.clone(),
            ctx,
            local_ty,
            out: String::new(),
            tmp: 0,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────

    fn fresh(&mut self) -> u32 { let t = self.tmp; self.tmp += 1; t }

    fn w(&mut self, s: &str) { self.out.push_str(s); self.out.push('\n'); }

    fn local_ptr(id: MirLocal) -> String { format!("%_l{}.ptr", id) }

    fn llty_of(&self, id: MirLocal) -> &str {
        self.local_ty.get(&id).map(|s| s.as_str()).unwrap_or("ptr")
    }

    // ── Operand loading ───────────────────────────────────────────────

    /// Emit a `load` for a local; return const values inline.
    /// Returns `(llvm_type, ssa_value)`.
    fn load(&mut self, op: &Operand) -> (String, String) {
        match op {
            Operand::Local(id) => {
                let lty = self.llty_of(*id).to_string();
                let v   = self.fresh();
                self.w(&format!("  %v{v} = load {lty}, ptr {}",
                    Self::local_ptr(*id)));
                (lty, format!("%v{}", v))
            }
            Operand::Const(c) => self.emit_const(c),
            Operand::Global(n) => ("ptr".into(), format!("@{}", llvm_fn_name(n))),
        }
    }

    fn emit_const(&mut self, c: &MirConst) -> (String, String) {
        match c {
            MirConst::Int(n)   => ("i64".into(),    n.to_string()),
            MirConst::Float(f) => ("double".into(), format!("{:e}", f)),
            MirConst::Bool(b)  => ("i1".into(),     if *b {"1".into()} else {"0".into()}),
            MirConst::Unit     => ("i8".into(),      "0".into()),

            MirConst::Str(s) | MirConst::Decimal(s) | MirConst::Uuid(s) => {
                let idx = self.ctx.intern_str(s);
                let len = s.len() + 1;
                let v   = self.fresh();
                self.w(&format!(
                    "  %v{v} = getelementptr [{len} x i8], ptr @.str.{idx}, i32 0, i32 0"
                ));
                ("ptr".into(), format!("%v{}", v))
            }
        }
    }

    // ── Statement emission ────────────────────────────────────────────

    fn emit_stmt(&mut self, stmt: MirStmt) {
        let MirStmt::Assign { dest, rvalue } = stmt;
        let dest_lty = self.llty_of(dest).to_string();
        let dest_ptr = Self::local_ptr(dest);
        let _is_unit  = dest_lty == "i8" && dest == 0; // rough Unit check for ret slot

        match rvalue {
            Rvalue::Use(op) => {
                let (lty, val) = self.load(&op);
                self.w(&format!("  store {lty} {val}, ptr {dest_ptr}"));
            }

            Rvalue::BinOp { op, lhs, rhs } => {
                let (lty, lv) = self.load(&lhs);
                let (_, rv)   = self.load(&rhs);
                let is_float  = lty == "double";
                let v         = self.fresh();
                let rty       = self.binop_instr(&op, &lty, &lv, &rv, is_float, v);
                self.w(&format!("  store {rty} %v{v}, ptr {dest_ptr}"));
            }

            Rvalue::UnOp { op, arg } => {
                let (aty, av) = self.load(&arg);
                let v = self.fresh();
                match op {
                    UnOp::Neg if aty == "double" => {
                        self.w(&format!("  %v{v} = fneg double {av}"));
                        self.w(&format!("  store double %v{v}, ptr {dest_ptr}"));
                    }
                    UnOp::Neg => {
                        self.w(&format!("  %v{v} = sub {aty} 0, {av}"));
                        self.w(&format!("  store {aty} %v{v}, ptr {dest_ptr}"));
                    }
                    UnOp::Not => {
                        self.w(&format!("  %v{v} = xor i1 {av}, 1"));
                        self.w(&format!("  store i1 %v{v}, ptr {dest_ptr}"));
                    }
                }
            }

            Rvalue::Call { func, args } => {
                self.emit_call_store(&func, &args, dest, &dest_lty, &dest_ptr);
            }

            Rvalue::Field { .. } => {
                // LLVM backend: field access on structs not yet supported — emit null.
                let v = self.fresh();
                self.w(&format!("  %v{v} = inttoptr i64 0 to ptr"));
                self.w(&format!("  store ptr %v{v}, ptr {dest_ptr}"));
            }

            Rvalue::Aggregate(_, _) => {
                let v = self.fresh();
                self.w(&format!("  %v{v} = inttoptr i64 0 to ptr"));
                self.w(&format!("  store ptr %v{v}, ptr {dest_ptr}"));
            }

            // Concurrency (`spawn`/`await`) not supported by the experimental
            // LLVM backend — emit null placeholders (the C backend is canonical).
            Rvalue::Spawn { .. } | Rvalue::Join { .. }
            | Rvalue::BoxSome { .. } | Rvalue::UnboxSome { .. } => {
                let v = self.fresh();
                self.w(&format!("  %v{v} = inttoptr i64 0 to ptr"));
                self.w(&format!("  store ptr %v{v}, ptr {dest_ptr}"));
            }
        }
    }

    /// Emit a binop instruction; return the LLVM result type string.
    fn binop_instr(
        &mut self,
        op: &BinOp, lty: &str, lv: &str, rv: &str, is_float: bool, v: u32,
    ) -> String {
        let name = format!("%v{}", v);
        let (line, rty) = match op {
            BinOp::Add  => (
                if is_float { format!("  {name} = fadd double {lv}, {rv}") }
                else        { format!("  {name} = add i64 {lv}, {rv}") },
                if is_float { "double" } else { "i64" },
            ),
            BinOp::Sub  => (
                if is_float { format!("  {name} = fsub double {lv}, {rv}") }
                else        { format!("  {name} = sub i64 {lv}, {rv}") },
                if is_float { "double" } else { "i64" },
            ),
            BinOp::Mul  => (
                if is_float { format!("  {name} = fmul double {lv}, {rv}") }
                else        { format!("  {name} = mul i64 {lv}, {rv}") },
                if is_float { "double" } else { "i64" },
            ),
            BinOp::Div  => (
                if is_float { format!("  {name} = fdiv double {lv}, {rv}") }
                else        { format!("  {name} = sdiv i64 {lv}, {rv}") },
                if is_float { "double" } else { "i64" },
            ),
            BinOp::Rem  => (format!("  {name} = srem i64 {lv}, {rv}"), "i64"),
            BinOp::Pow  => {
                self.ctx.declare_extern(
                    "certo_pow".into(), "i64".into(),
                    vec!["i64".into(), "i64".into()]);
                (format!("  {name} = call i64 @certo_pow(i64 {lv}, i64 {rv})"), "i64")
            }
            BinOp::Eq    => (
                if is_float { format!("  {name} = fcmp oeq double {lv}, {rv}") }
                else        { format!("  {name} = icmp eq {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::NotEq => (
                if is_float { format!("  {name} = fcmp one double {lv}, {rv}") }
                else        { format!("  {name} = icmp ne {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::Lt    => (
                if is_float { format!("  {name} = fcmp olt double {lv}, {rv}") }
                else        { format!("  {name} = icmp slt {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::LtEq  => (
                if is_float { format!("  {name} = fcmp ole double {lv}, {rv}") }
                else        { format!("  {name} = icmp sle {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::Gt    => (
                if is_float { format!("  {name} = fcmp ogt double {lv}, {rv}") }
                else        { format!("  {name} = icmp sgt {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::GtEq  => (
                if is_float { format!("  {name} = fcmp oge double {lv}, {rv}") }
                else        { format!("  {name} = icmp sge {lty} {lv}, {rv}") },
                "i1",
            ),
            BinOp::And         => (format!("  {name} = and i1 {lv}, {rv}"), "i1"),
            BinOp::Or          => (format!("  {name} = or i1 {lv}, {rv}"),  "i1"),
            BinOp::NullCoalesce => {
                self.ctx.declare_extern(
                    "certo_coalesce".into(), "ptr".into(),
                    vec!["ptr".into(), "ptr".into()]);
                (format!("  {name} = call ptr @certo_coalesce(ptr {lv}, ptr {rv})"), "ptr")
            }
            BinOp::Concat => {
                self.ctx.declare_extern(
                    "certo_text_concat".into(), "ptr".into(),
                    vec!["ptr".into(), "ptr".into()]);
                (format!("  {name} = call ptr @certo_text_concat(ptr {lv}, ptr {rv})"), "ptr")
            }
        };
        self.w(&line);
        rty.into()
    }

    /// Emit a call and store the result to `dest_ptr`.
    fn emit_call_store(
        &mut self,
        func:     &Operand,
        args:     &[Operand],
        _dest:     MirLocal,
        dest_lty: &str,
        dest_ptr: &str,
    ) {
        let arg_pairs: Vec<(String, String)> = args.iter().map(|a| self.load(a)).collect();
        let arg_str = arg_pairs.iter()
            .map(|(t, v)| format!("{t} {v}"))
            .collect::<Vec<_>>().join(", ");

        let callee = match func {
            Operand::Global(n) => {
                let ln = llvm_fn_name(n);
                let arg_tys: Vec<String> = arg_pairs.iter().map(|(t, _)| t.clone()).collect();
                self.ctx.declare_extern(ln.clone(), dest_lty.to_string(), arg_tys);
                format!("@{}", ln)
            }
            _ => self.load(func).1,
        };

        let is_void = dest_lty == "void" || dest_lty == "i8";
        if is_void {
            self.w(&format!("  call void {callee}({arg_str})"));
            self.w(&format!("  store i8 0, ptr {dest_ptr}"));
        } else {
            let v = self.fresh();
            self.w(&format!("  %v{v} = call {dest_lty} {callee}({arg_str})"));
            self.w(&format!("  store {dest_lty} %v{v}, ptr {dest_ptr}"));
        }
    }

    // ── Terminator emission ───────────────────────────────────────────

    fn emit_terminator(&mut self, term: Terminator, ret_llty: &str) {
        match term {
            Terminator::Goto(bb) => {
                self.w(&format!("  br label %bb{bb}"));
            }

            Terminator::If { cond, true_bb, false_bb } => {
                let (_, cv) = self.load(&cond);
                self.w(&format!("  br i1 {cv}, label %bb{true_bb}, label %bb{false_bb}"));
            }

            Terminator::Return(op) => {
                if ret_llty == "void" {
                    self.w("  ret void");
                } else {
                    let (ty, v) = self.load(&op);
                    self.w(&format!("  ret {ty} {v}"));
                }
            }

            Terminator::Unreachable => {
                self.w("  unreachable");
            }

            Terminator::Call { func, args, dest, next } => {
                let dest_lty = self.llty_of(dest).to_string();
                let dest_ptr = Self::local_ptr(dest);
                self.emit_call_store(&func, &args, dest, &dest_lty, &dest_ptr);
                self.w(&format!("  br label %bb{next}"));
            }

            Terminator::Switch { discr, targets, otherwise } => {
                let (dty, dv) = self.load(&discr);
                let cases: String = targets.iter().map(|(tgt, bb)| {
                    let val = match tgt {
                        SwitchTarget::Int(n)  => format!("{dty} {n}"),
                        SwitchTarget::Bool(b) => format!("i1 {}", if *b { 1 } else { 0 }),
                    };
                    format!("    {val}, label %bb{bb}")
                }).collect::<Vec<_>>().join("\n");
                self.w(&format!(
                    "  switch {dty} {dv}, label %bb{otherwise} [\n{cases}\n  ]"
                ));
            }
        }
    }

    // ── Top-level ─────────────────────────────────────────────────────

    pub fn emit(mut self) -> String {
        let fname    = llvm_fn_name(&self.ast_fn.name.node);
        let ret_llty = ast_ret_ty_to_llvm(
            self.ast_fn.ret_ty.as_ref().map(|s| &s.node)
        );

        // LLVM parameter list (from AST, not HIR)
        let params: String = self.ast_fn.params.iter()
            .map(|p| {
                let lty = ast_ty_to_llvm(&p.ty.node);
                // Find the MIR local id for this param by name
                let id  = self.mir_fn.locals.iter()
                    .find(|l| l.name == p.name.node)
                    .map(|l| l.id)
                    .unwrap_or(0);
                format!("{lty} %p{id}")
            })
            .collect::<Vec<_>>()
            .join(", ");

        self.w(&format!("define {ret_llty} @{fname}({params}) {{"));
        self.w("entry:");

        // alloca every local
        for local in self.mir_fn.locals.clone() {
            let lty = self.llty_of(local.id).to_string();
            self.w(&format!("  {} = alloca {lty}", Self::local_ptr(local.id)));
        }

        // store params into their alloca slots
        for param in self.ast_fn.params.clone() {
            let lty = ast_ty_to_llvm(&param.ty.node);
            if let Some(ml) = self.mir_fn.locals.iter().find(|l| l.name == param.name.node) {
                let id = ml.id;
                self.w(&format!("  store {lty} %p{id}, ptr {}", Self::local_ptr(id)));
            }
        }

        self.w("  br label %bb0");
        self.w("");

        // emit basic blocks
        for bb in self.mir_fn.blocks.clone() {
            self.w(&format!("bb{}:", bb.id));
            for stmt in bb.stmts { self.emit_stmt(stmt); }
            match bb.terminator {
                Some(t) => self.emit_terminator(t, &ret_llty.clone()),
                None    => self.w("  unreachable"),
            }
            self.w("");
        }

        self.w("}");
        self.out
    }
}
