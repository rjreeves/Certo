/// HIR-based lint pass for Certo.
///
/// Checks performed:
///   L001  unused parameter        — parameter never read in the function body
///   L002  unused variable         — val/var declared but never read
///   L003  assigned but never read — variable written after declaration but the
///                                   written value is never subsequently read
///   L004  unreachable statement   — statement after a call to panic/todo/unreachable
///   L005  guard clause always true/false — guard condition is a literal bool

use std::collections::{HashMap, HashSet};
use std::path::Path;
use certo_ast::span::Span;
use certo_hir::{HirFn, HirExpr, HirExprKind, HirStmt, HirItem, LocalId, UnOp, lower_module};
use certo_ast::module::Module;

// ------------------------------------------------------------------ //
// Public entry point
// ------------------------------------------------------------------ //

pub fn lint_hir(module: &Module, path: &Path, src: &str, color: bool) -> usize {
    let hir = match lower_module(module) {
        Ok(h) => h,
        Err(_) => {
            // HIR lowering failed (e.g. unresolved names); fall back gracefully.
            return 0;
        }
    };

    let mut total = 0;
    for item in &hir.items {
        if let HirItem::Fn(f) = item {
            total += lint_fn(f, path, src, color);
        }
    }
    total
}

// ------------------------------------------------------------------ //
// Per-function lint
// ------------------------------------------------------------------ //

fn lint_fn(f: &HirFn, path: &Path, src: &str, color: bool) -> usize {
    let body = match &f.body {
        Some(b) => b,
        None => return 0,
    };

    // ── Pass 1: collect all reads ─────────────────────────────────────
    let mut reads: HashSet<LocalId> = HashSet::new();
    collect_reads(body, &mut reads);

    let mut warnings = 0;

    // ── L001: unused parameters ───────────────────────────────────────
    for param in &f.params {
        if param.name.starts_with('_') { continue; }
        if !reads.contains(&param.local) {
            let msg = format!("unused parameter `{}`", param.name);
            emit(path, src, param.span, "L001", &msg, color);
            warnings += 1;
        }
    }

    // ── Pass 2: walk stmts for unused vars and unreachable code ───────
    warnings += lint_block_stmts(body, path, src, &reads, color);

    warnings
}

// ------------------------------------------------------------------ //
// Block statement lints (L002, L003, L004)
// ------------------------------------------------------------------ //

fn lint_block_stmts(expr: &HirExpr, path: &Path, src: &str,
                    reads: &HashSet<LocalId>, color: bool) -> usize {
    let HirExprKind::Block { stmts, tail } = &expr.kind else {
        // Recurse into nested blocks.
        return recurse_lint(expr, path, src, reads, color);
    };

    let mut count = 0;
    let mut terminal = false;

    // Track which locals have been written since their last read.
    // Map: local → (name, span_of_last_write, write_was_declaration)
    let mut pending_writes: HashMap<LocalId, (String, Span, bool)> = HashMap::new();

    for stmt in stmts {
        // L004: unreachable after terminal call.
        if terminal {
            let span = stmt_span(stmt);
            emit(path, src, span, "L004", "unreachable statement", color);
            count += 1;
            break;
        }

        match stmt {
            HirStmt::Let { local, name, init, .. } => {
                // Check if the init expression itself contains a terminal call.
                if is_terminal_call(init) { terminal = true; }

                // Recurse into the init expression for nested blocks.
                count += lint_block_stmts(init, path, src, reads, color);

                // L002: if this local is never read anywhere in the function.
                if !name.starts_with('_') && !reads.contains(local) {
                    emit(path, src, init.span, "L002",
                        &format!("unused variable `{}`", name), color);
                    count += 1;
                } else {
                    // Track as a pending write (may be overwritten before read).
                    pending_writes.insert(*local, (name.clone(), init.span, true));
                }
            }

            HirStmt::Assign { local, value } => {
                if is_terminal_call(value) { terminal = true; }
                count += lint_block_stmts(value, path, src, reads, color);

                // L003: if previous write to this local was never read before this
                // new write, the old value was thrown away.
                // We only flag non-declaration writes (val/var re-assignments).
                if let Some((name, prev_span, was_decl)) = pending_writes.get(local) {
                    if !name.starts_with('_') && !was_decl {
                        emit(path, src, *prev_span, "L003",
                            &format!("value assigned to `{}` is never read", name), color);
                        count += 1;
                    }
                }

                // Record this write as the new pending write.
                // Re-read the name from pending_writes or fall back to the local id.
                let name = pending_writes.get(local)
                    .map(|(n, _, _)| n.clone())
                    .unwrap_or_else(|| format!("_l{}", local));
                pending_writes.insert(*local, (name, value.span, false));
            }

            HirStmt::Expr(e) => {
                // L005: guard clause with a literal bool condition.
                if let Some((cond, then_expr)) = as_guard_pattern(e) {
                    match &cond.kind {
                        HirExprKind::Bool(true) => {
                            emit(path, src, e.span, "L005",
                                "guard condition is always true — this guard clause can never fire",
                                color);
                            count += 1;
                        }
                        HirExprKind::Bool(false) => {
                            emit(path, src, e.span, "L005",
                                "guard condition is always false — this guard always fires; code after is unreachable",
                                color);
                            count += 1;
                            terminal = true;
                        }
                        _ => {
                            // If the then_expr (the early-return branch) is terminal,
                            // treat the guard as a terminal for L004 purposes.
                            if is_terminal_call(then_expr) { terminal = true; }
                        }
                    }
                } else if is_terminal_call(e) {
                    terminal = true;
                }
                count += lint_block_stmts(e, path, src, reads, color);

                // A read of a local within this expression clears its pending write.
                let mut local_reads = HashSet::new();
                collect_reads(e, &mut local_reads);
                for id in &local_reads {
                    pending_writes.remove(id);
                }
            }
        }

        // After each statement, remove locals that were read by subsequent expressions.
        // (We do a forward scan here: if the local appears in reads at all, assume it
        //  may be read later in the block — this is conservative but avoids false positives
        //  from cross-branch analysis which would need full dataflow.)
    }

    // Recurse into the tail expression.
    count += lint_block_stmts(tail, path, src, reads, color);

    count
}

/// Recurse into non-block expressions to find nested blocks.
fn recurse_lint(expr: &HirExpr, path: &Path, src: &str,
                reads: &HashSet<LocalId>, color: bool) -> usize {
    let mut count = 0;
    match &expr.kind {
        HirExprKind::Block { .. } => {
            count += lint_block_stmts(expr, path, src, reads, color);
        }
        HirExprKind::If { cond, then_expr, else_expr } => {
            count += lint_block_stmts(cond,      path, src, reads, color);
            count += lint_block_stmts(then_expr, path, src, reads, color);
            count += lint_block_stmts(else_expr, path, src, reads, color);
        }
        HirExprKind::Call { func, args } => {
            count += lint_block_stmts(func, path, src, reads, color);
            for a in args { count += lint_block_stmts(a, path, src, reads, color); }
        }
        HirExprKind::BinOp { lhs, rhs, .. } => {
            count += lint_block_stmts(lhs, path, src, reads, color);
            count += lint_block_stmts(rhs, path, src, reads, color);
        }
        HirExprKind::UnOp { arg, .. } => {
            count += lint_block_stmts(arg, path, src, reads, color);
        }
        HirExprKind::Field { base, .. } => {
            count += lint_block_stmts(base, path, src, reads, color);
        }
        HirExprKind::Record(fields) => {
            for (_, e) in fields { count += lint_block_stmts(e, path, src, reads, color); }
        }
        HirExprKind::Tuple(elems) | HirExprKind::List(elems) => {
            for e in elems { count += lint_block_stmts(e, path, src, reads, color); }
        }
        HirExprKind::Match { scrutinee, arms } => {
            count += lint_block_stmts(scrutinee, path, src, reads, color);
            for arm in arms {
                count += lint_block_stmts(&arm.body, path, src, reads, color);
            }
        }
        HirExprKind::For { iter, body, .. } => {
            count += lint_block_stmts(iter, path, src, reads, color);
            count += lint_block_stmts(body, path, src, reads, color);
        }
        HirExprKind::While { cond, body } => {
            count += lint_block_stmts(cond, path, src, reads, color);
            count += lint_block_stmts(body, path, src, reads, color);
        }
        HirExprKind::Lambda { body, .. } => {
            count += lint_block_stmts(body, path, src, reads, color);
        }
        HirExprKind::Try(e) | HirExprKind::Unsafe(e) => {
            count += lint_block_stmts(e, path, src, reads, color);
        }
        _ => {}
    }
    count
}

// ------------------------------------------------------------------ //
// Collect all LocalId reads in an expression subtree
// ------------------------------------------------------------------ //

fn collect_reads(expr: &HirExpr, out: &mut HashSet<LocalId>) {
    match &expr.kind {
        HirExprKind::Local(id) => { out.insert(*id); }

        HirExprKind::Block { stmts, tail } => {
            for stmt in stmts {
                match stmt {
                    HirStmt::Let  { init, .. }  => collect_reads(init, out),
                    HirStmt::Assign { value, .. } => collect_reads(value, out),
                    HirStmt::Expr(e)             => collect_reads(e, out),
                }
            }
            collect_reads(tail, out);
        }

        HirExprKind::Call { func, args } => {
            collect_reads(func, out);
            for a in args { collect_reads(a, out); }
        }
        HirExprKind::BinOp { lhs, rhs, .. } => {
            collect_reads(lhs, out); collect_reads(rhs, out);
        }
        HirExprKind::UnOp { arg, .. } => collect_reads(arg, out),
        HirExprKind::Field { base, .. } => collect_reads(base, out),
        HirExprKind::Record(fields) => {
            for (_, e) in fields { collect_reads(e, out); }
        }
        HirExprKind::Tuple(elems) | HirExprKind::List(elems) => {
            for e in elems { collect_reads(e, out); }
        }
        HirExprKind::If { cond, then_expr, else_expr } => {
            collect_reads(cond, out);
            collect_reads(then_expr, out);
            collect_reads(else_expr, out);
        }
        HirExprKind::Match { scrutinee, arms } => {
            collect_reads(scrutinee, out);
            for arm in arms { collect_reads(&arm.body, out); }
        }
        HirExprKind::For { iter, body, binding, .. } => {
            // `binding` is written, not read — don't add it here.
            // But if the body reads it, that read is captured by body traversal.
            let _ = binding;
            collect_reads(iter, out);
            collect_reads(body, out);
        }
        HirExprKind::While { cond, body } => {
            collect_reads(cond, out); collect_reads(body, out);
        }
        HirExprKind::Lambda { body, .. } => collect_reads(body, out),
        HirExprKind::Try(e) | HirExprKind::Unsafe(e) => collect_reads(e, out),
        _ => {}
    }
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

/// Detect the HIR pattern produced by `guard cond else e`:
///   Block { stmts: [Expr(If { cond: UnOp(Not, inner), then_expr, else_expr: Unit })], tail: Unit }
///
/// Returns `Some((inner_cond, then_expr))` when matched.
fn as_guard_pattern<'a>(expr: &'a HirExpr) -> Option<(&'a HirExpr, &'a HirExpr)> {
    let HirExprKind::Block { stmts, tail } = &expr.kind else { return None; };
    if stmts.len() != 1 { return None; }
    if !matches!(tail.kind, HirExprKind::Unit) { return None; }
    let HirStmt::Expr(if_expr) = &stmts[0] else { return None; };
    let HirExprKind::If { cond, then_expr, else_expr } = &if_expr.kind else { return None; };
    if !matches!(else_expr.kind, HirExprKind::Unit) { return None; }
    let HirExprKind::UnOp { op: UnOp::Not, arg } = &cond.kind else { return None; };
    Some((arg, then_expr)
    )
}

/// True if the expression is a direct call to `panic`, `todo`, or `unreachable`.
fn is_terminal_call(expr: &HirExpr) -> bool {
    if let HirExprKind::Call { func, .. } = &expr.kind {
        if let HirExprKind::Global(name) = &func.kind {
            let leaf = name.rsplit('.').next().unwrap_or(name.as_str());
            return matches!(leaf, "panic" | "todo" | "unreachable");
        }
    }
    false
}

fn stmt_span(stmt: &HirStmt) -> Span {
    match stmt {
        HirStmt::Let    { init, .. }  => init.span,
        HirStmt::Assign { value, .. } => value.span,
        HirStmt::Expr(e)             => e.span,
    }
}

/// Emit a lint warning to stderr.
fn emit(path: &Path, src: &str, span: Span, code: &str, msg: &str, color: bool) {
    let line = src[..span.start as usize].chars().filter(|&c| c == '\n').count() + 1;
    let col  = span.start as usize
        - src[..span.start as usize].rfind('\n').map(|p| p + 1).unwrap_or(0)
        + 1;

    // Source snippet: the line containing the span.
    let line_start = src[..span.start as usize].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let line_end   = src[span.start as usize..].find('\n')
        .map(|p| span.start as usize + p)
        .unwrap_or(src.len());
    let snippet = src[line_start..line_end].trim_end();

    // Underline: carets from col to end of span (clamped to line end).
    let span_end   = (span.end as usize).min(line_end);
    let underline_len = if span_end > span.start as usize {
        span_end - span.start as usize
    } else {
        1
    };
    let indent = " ".repeat(col - 1);
    let carets = "^".repeat(underline_len);

    if color {
        eprintln!("\x1b[1m{}:{}:{}: \x1b[33mwarning[{}]\x1b[0m\x1b[1m: {}\x1b[0m",
            path.display(), line, col, code, msg);
        eprintln!("   {}", snippet);
        eprintln!("   \x1b[33m{}{}\x1b[0m", indent, carets);
    } else {
        eprintln!("{}:{}:{}: warning[{}]: {}", path.display(), line, col, code, msg);
        eprintln!("   {}", snippet);
        eprintln!("   {}{}", indent, carets);
    }
    eprintln!();
}
