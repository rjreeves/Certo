//! Structural `==` for records, sum types, lists, tuples and options (BACKLOG item 437).
//!
//! The C backend generates a `certo_eq_<Type>` per type; this does the same in IR. Each distinct
//! type gets one function `i1 @certo_eq_N(ptr a, ptr b)` that compares the two values slot by
//! slot, calling the function for a nested type, so a recursive type compares recursively.

use certo_typeck::Ty;

use crate::ctx::ModuleCtx;
use crate::ty::llvm_ty;

/// Can `==` on this type be compiled structurally? (Text is compared by the caller.)
pub fn is_structural(ctx: &ModuleCtx, ty: &Ty) -> bool {
    match ty {
        Ty::Named { name, .. } => ctx.records.contains_key(name) || ctx.enums.contains_key(name),
        Ty::List(_) | Ty::Option(_) => true,
        Ty::Tuple(ts) => !ts.is_empty(),
        _ => false,
    }
}

/// The symbol of the equality function for `ty`, generating it (and those it needs) on first use.
pub fn eq_symbol(ctx: &mut ModuleCtx, ty: &Ty) -> Result<String, String> {
    let key = format!("{ty:?}");
    if let Some(sym) = ctx.eq_names.get(&key) {
        return Ok(sym.clone());
    }
    let sym = format!("certo_eq_{}", ctx.eq_names.len());
    // Registered before the body is built, so a type that contains itself finds its own symbol.
    ctx.eq_names.insert(key, sym.clone());
    let body = build(ctx, &sym, ty)?;
    ctx.eq_bodies.push(body);
    Ok(sym)
}

struct Fb {
    out: String,
    n: u32,
}

impl Fb {
    fn w(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn t(&mut self) -> u32 {
        self.n += 1;
        self.n
    }
}

/// `i1` SSA value comparing `a` and `b`, two values of `ty` (already loaded as `llty(ty)`).
fn compare(ctx: &mut ModuleCtx, f: &mut Fb, ty: &Ty, a: &str, b: &str) -> Result<String, String> {
    let l = llvm_ty(ty);
    let r = f.t();
    match ty {
        Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt | Ty::Bool | Ty::Char => {
            f.w(&format!("  %c{r} = icmp eq {l} {a}, {b}"));
        }
        Ty::Float | Ty::Float32 => f.w(&format!("  %c{r} = fcmp oeq {l} {a}, {b}")),
        Ty::Unit => f.w(&format!("  %c{r} = icmp eq i8 0, 0")),
        Ty::Text | Ty::BoundedText(_) => {
            ctx.declare_extern("certo_rt_text_eq".into(), "zeroext i1".into(), vec!["ptr".into(), "ptr".into()]);
            f.w(&format!("  %c{r} = call zeroext i1 @certo_rt_text_eq(ptr {a}, ptr {b})"));
        }
        other if is_structural(ctx, other) => {
            let sym = eq_symbol(ctx, other)?;
            f.w(&format!("  %c{r} = call zeroext i1 @{sym}(ptr {a}, ptr {b})"));
        }
        other => return Err(format!("`==` on a value of type `{}` is not supported by the LLVM backend", describe(other))),
    }
    Ok(format!("%c{r}"))
}

fn describe(ty: &Ty) -> String {
    match ty {
        Ty::Var(_) | Ty::Error => "a generic or unknown type".into(),
        Ty::Named { name, .. } => name.clone(),
        other => format!("{other:?}"),
    }
}

/// Load a `ty` from `base + slot * 8`.
fn load_slot(f: &mut Fb, base: &str, slot: usize, ty: &Ty) -> String {
    let l = llvm_ty(ty);
    let g = f.t();
    f.w(&format!("  %g{g} = getelementptr i8, ptr {base}, i64 {}", slot * 8));
    let v = f.t();
    f.w(&format!("  %s{v} = load {l}, ptr %g{g}"));
    format!("%s{v}")
}

/// Compare the given `(slot, type)` pairs of `%a` and `%b`; on the first difference jump to `ne`,
/// when all are equal fall through to a new block `done_label`.
fn compare_slots(ctx: &mut ModuleCtx, f: &mut Fb, slots: &[(usize, Ty)], ne: &str, done_label: &str) -> Result<(), String> {
    for (slot, ty) in slots {
        let a = load_slot(f, "%a", *slot, ty);
        let b = load_slot(f, "%b", *slot, ty);
        let c = compare(ctx, f, ty, &a, &b)?;
        let next = f.t();
        f.w(&format!("  br i1 {c}, label %n{next}, label %{ne}"));
        f.w(&format!("n{next}:"));
    }
    f.w(&format!("  br label %{done_label}"));
    Ok(())
}

fn build(ctx: &mut ModuleCtx, sym: &str, ty: &Ty) -> Result<String, String> {
    let mut f = Fb { out: String::new(), n: 0 };
    f.w(&format!("define internal zeroext i1 @{sym}(ptr %a, ptr %b) {{"));
    f.w("entry:");
    match ty {
        Ty::Named { name, .. } if ctx.records.contains_key(name) => {
            let tys = ctx.record_tys.get(name).cloned().unwrap_or_default();
            let slots: Vec<(usize, Ty)> = tys.into_iter().enumerate().collect();
            compare_slots(ctx, &mut f, &slots, "ne", "yes")?;
        }
        Ty::Tuple(ts) => {
            let slots: Vec<(usize, Ty)> = ts.iter().cloned().enumerate().collect();
            compare_slots(ctx, &mut f, &slots, "ne", "yes")?;
        }
        Ty::Named { name, .. } => {
            // Tag in slot 0, then the variant's payload from slot 1.
            let variants = ctx.variant_tys.get(name).cloned().unwrap_or_default();
            f.w("  %ta = load i64, ptr %a");
            f.w("  %tb = load i64, ptr %b");
            f.w("  %same = icmp eq i64 %ta, %tb");
            f.w("  br i1 %same, label %tags, label %ne");
            f.w("tags:");
            let with_fields: Vec<_> = variants.iter().filter(|(_, tys)| !tys.is_empty()).collect();
            let cases = with_fields.iter().map(|(tag, _)| format!("    i64 {tag}, label %v{tag}")).collect::<Vec<_>>().join("\n");
            f.w(&format!("  switch i64 %ta, label %yes [\n{cases}\n  ]"));
            for (tag, tys) in &with_fields {
                f.w(&format!("v{tag}:"));
                let slots: Vec<(usize, Ty)> = tys.iter().cloned().enumerate().map(|(i, t)| (1 + i, t)).collect();
                compare_slots(ctx, &mut f, &slots, "ne", "yes")?;
            }
        }
        Ty::Option(inner) => {
            f.w("  %na = icmp eq ptr %a, null");
            f.w("  %nb = icmp eq ptr %b, null");
            f.w("  %both = and i1 %na, %nb");
            f.w("  br i1 %both, label %yes, label %chk");
            f.w("chk:");
            f.w("  %either = or i1 %na, %nb");
            f.w("  br i1 %either, label %ne, label %cmp");
            f.w("cmp:");
            let a = load_slot(&mut f, "%a", 0, inner);
            let b = load_slot(&mut f, "%b", 0, inner);
            let c = compare(ctx, &mut f, inner, &a, &b)?;
            f.w(&format!("  ret i1 {c}"));
        }
        Ty::List(elem) => {
            // { len, cap, data }: the length is slot 0 and the data pointer is at offset 16.
            f.w("  %i = alloca i64");
            f.w("  %la = load i64, ptr %a");
            f.w("  %lb = load i64, ptr %b");
            f.w("  %leneq = icmp eq i64 %la, %lb");
            f.w("  br i1 %leneq, label %start, label %ne");
            f.w("start:");
            f.w("  %pa = getelementptr i8, ptr %a, i64 16");
            f.w("  %da = load ptr, ptr %pa");
            f.w("  %pb = getelementptr i8, ptr %b, i64 16");
            f.w("  %db = load ptr, ptr %pb");
            f.w("  store i64 0, ptr %i");
            f.w("  br label %loop");
            f.w("loop:");
            f.w("  %iv = load i64, ptr %i");
            f.w("  %end = icmp sge i64 %iv, %la");
            f.w("  br i1 %end, label %yes, label %body");
            f.w("body:");
            f.w("  %off = mul i64 %iv, 8");
            f.w("  %ea = getelementptr i8, ptr %da, i64 %off");
            f.w("  %eb = getelementptr i8, ptr %db, i64 %off");
            let l = llvm_ty(elem);
            f.w(&format!("  %xa = load {l}, ptr %ea"));
            f.w(&format!("  %xb = load {l}, ptr %eb"));
            let c = compare(ctx, &mut f, elem, "%xa", "%xb")?;
            f.w(&format!("  br i1 {c}, label %next, label %ne"));
            f.w("next:");
            f.w("  %nx = add i64 %iv, 1");
            f.w("  store i64 %nx, ptr %i");
            f.w("  br label %loop");
        }
        other => return Err(format!("`==` on a value of type `{}` is not supported by the LLVM backend", describe(other))),
    }
    f.w("yes:");
    f.w("  ret i1 1");
    f.w("ne:");
    f.w("  ret i1 0");
    f.w("}");
    Ok(f.out)
}
