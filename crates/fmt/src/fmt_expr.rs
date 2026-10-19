use certo_ast::expr::{Expr, ExpectMatcher, Stmt, Lit, FStringPart, UnOp, BinOp, Arg, MatchArm, LambdaParam};
use certo_ast::pattern::Pattern;
use crate::printer::{ind, group, binop_str};
use crate::fmt_type::fmt_type;

// ------------------------------------------------------------------ //
// Operator precedence
//
// The AST has no "parenthesised" node — `(a + b) * c` parses to a `Mul` whose
// left child is an `Add`, and the parentheses are gone. The printer therefore
// has to put them back wherever the grammar needs them, or the formatted
// program computes something else (BACKLOG item 403).
//
// These levels mirror the parser's own ladder in `crates/parser/src/
// parse_expr.rs` (low → high) and must be kept in step with it. `fmt_operand`
// parenthesises a child whenever it binds more loosely than its position
// allows.
// ------------------------------------------------------------------ //

const P_PIPE: u8 = 1;
const P_OR: u8 = 2;
const P_AND: u8 = 3;
const P_COALESCE: u8 = 4;
const P_NOT: u8 = 5;
const P_CMP: u8 = 6;
const P_RANGE: u8 = 7;
const P_ADD: u8 = 8;
const P_MUL: u8 = 9;
const P_POW: u8 = 10;
const P_UNARY: u8 = 11;
const P_POSTFIX: u8 = 12;
const P_ATOM: u8 = 13;

/// How tightly an expression's own printed form binds. `0` means it is
/// open-ended (a trailing `else`/lambda body swallows whatever follows), so it
/// needs parentheses whenever it is an operand of anything.
fn prec(expr: &Expr) -> u8 {
    match expr {
        Expr::Pipe { .. } => P_PIPE,
        Expr::BinOp { op, .. } => match op {
            BinOp::Or => P_OR,
            BinOp::And => P_AND,
            BinOp::NullCoalesce => P_COALESCE,
            BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq | BinOp::In => P_CMP,
            BinOp::RangeInclusive | BinOp::RangeExclusive => P_RANGE,
            BinOp::Add | BinOp::Sub | BinOp::Concat => P_ADD,
            BinOp::Mul | BinOp::Div | BinOp::Rem => P_MUL,
            BinOp::Pow => P_POW,
        },
        Expr::UnOp { op: UnOp::Not, .. } => P_NOT,
        Expr::UnOp { op: UnOp::Neg, .. } => P_UNARY,
        // `await x` / `spawn x` are prefix forms: fine as a binary operand, but
        // `await x.y` means `await (x.y)`, so they need parens as a receiver.
        Expr::Await { .. } | Expr::Spawn { .. } => P_UNARY,
        Expr::Field { .. } | Expr::SafeField { .. } | Expr::Try { .. } | Expr::App { .. }
        | Expr::Age { .. } | Expr::ExpectAssertion { .. } => P_POSTFIX,
        // Open-ended: the last thing they print is a full expression.
        Expr::If { .. } | Expr::Lambda { .. } | Expr::Guard { .. } | Expr::Require { .. }
        | Expr::Ascribe { .. } => 0,
        _ => P_ATOM,
    }
}

/// `(left, right)` — the loosest precedence each operand may have without
/// parentheses. Left-associative operators allow an equal-precedence left
/// operand and demand a tighter right one (`a - (b - c)` keeps its parens,
/// `a - b - c` doesn't); `**` and `??` are right-associative, so the reverse.
fn binop_operand_prec(op: &BinOp) -> (u8, u8) {
    match op {
        BinOp::Or => (P_OR, P_AND),
        BinOp::And => (P_AND, P_NOT),
        BinOp::NullCoalesce => (P_NOT, P_COALESCE),
        BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq | BinOp::In => (P_CMP, P_RANGE),
        BinOp::RangeInclusive | BinOp::RangeExclusive => (P_ADD, P_ADD),
        BinOp::Add | BinOp::Sub | BinOp::Concat => (P_ADD, P_MUL),
        BinOp::Mul | BinOp::Div | BinOp::Rem => (P_MUL, P_POW),
        BinOp::Pow => (P_UNARY, P_POW),
    }
}

/// Print `expr` as an operand that must bind at least as tightly as `min`.
fn fmt_operand(expr: &Expr, min: u8, indent: usize) -> String {
    let s = fmt_expr(expr, indent);
    if prec(expr) < min { format!("({})", s) } else { s }
}

/// Print `expr` as the receiver of a postfix form (`.field`, `?`, a call).
fn fmt_receiver(expr: &Expr, indent: usize) -> String {
    fmt_operand(expr, P_POSTFIX, indent)
}

pub fn fmt_expr(expr: &Expr, indent: usize) -> String {
    match expr {
        Expr::Lit { value, .. } => fmt_lit(value),

        Expr::Path { path, .. } =>
            path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."),

        Expr::App { func, args, .. } => {
            let f = fmt_receiver(&func.node, indent);
            let formatted_args: Vec<String> = args.iter().map(|a| fmt_arg(a, indent)).collect();
            let args_str = group(&formatted_args, "(", ")", ", ", indent, false);
            format!("{}{}", f, args_str)
        }

        Expr::Pipe { left, right, .. } => {
            let l = fmt_operand(&left.node, P_PIPE, indent);
            let r = fmt_operand(&right.node, P_OR, indent);
            let inline = format!("{} |> {}", l, r);
            if inline.len() <= 80 {
                inline
            } else {
                format!("{}\n{}|> {}", l, ind(indent + 1), r)
            }
        }

        Expr::BinOp { op, left, right, .. } => {
            let (lmin, rmin) = binop_operand_prec(op);
            let l = fmt_operand(&left.node, lmin, indent);
            let r = fmt_operand(&right.node, rmin, indent);
            format!("{} {} {}", l, binop_str(op), r)
        }

        Expr::UnOp { op, expr, .. } => match op {
            UnOp::Neg => {
                let e = fmt_operand(&expr.node, P_UNARY, indent);
                // `--x` would read as the start of a comment-like token; keep it apart.
                if e.starts_with('-') { format!("-({})", e) } else { format!("-{}", e) }
            }
            UnOp::Not => format!("not {}", fmt_operand(&expr.node, P_NOT, indent)),
        },

        Expr::Field { expr, field, .. } =>
            format!("{}.{}", fmt_receiver(&expr.node, indent), field.node),

        Expr::SafeField { expr, field, .. } =>
            format!("{}?.{}", fmt_receiver(&expr.node, indent), field.node),

        Expr::If { cond, then_expr, else_expr, .. } => {
            let c = fmt_expr(&cond.node, indent);
            let t = fmt_expr(&then_expr.node, indent + 1);
            let e = fmt_expr(&else_expr.node, indent + 1);
            let inline = format!("if {} then {} else {}", c, t, e);
            if inline.len() <= 80 {
                inline
            } else {
                format!(
                    "if {} then\n{}{}\n{}else\n{}{}",
                    c,
                    ind(indent + 1), t,
                    ind(indent),
                    ind(indent + 1), e,
                )
            }
        }

        Expr::Match { scrutinee, arms, .. } => {
            let s = fmt_expr(&scrutinee.node, indent);
            let arms_str = arms.iter().map(|a| fmt_arm(a, indent + 1)).collect::<Vec<_>>().join("\n");
            format!("match {} {{\n{}\n{}}}", s, arms_str, ind(indent))
        }

        Expr::Block { stmts, .. } => {
            if stmts.is_empty() { return "{}".to_string(); }
            let body = stmts.iter().map(|s| format!("{}{}", ind(indent + 1), fmt_stmt(s, indent + 1)))
                .collect::<Vec<_>>().join("\n");
            format!("{{\n{}\n{}}}", body, ind(indent))
        }

        Expr::Lambda { params, body, .. } => {
            // BACKLOG item 203 — a lambda synthesized by the parser's own
            // leading-dot shorthand desugar (`.lineTotal` -> `(__dot) =>
            // __dot.lineTotal`, `crates/parser/src/parse_expr.rs`) round-
            // trips back to its original shorthand spelling here, instead
            // of printing the internal `__dot` parameter name into the
            // user's own source — confirmed directly this would otherwise
            // happen (`certo fmt` turning `.lineTotal` into `__dot =>
            // __dot.lineTotal`), a real, visible leak of an implementation
            // detail a user never wrote. Only ever matches the *exact*
            // shape the parser's own desugar produces (one unannotated
            // param literally named `__dot`, whose body is a direct field
            // access or single call rooted at a reference to that same
            // param) — anything else prints as an ordinary lambda.
            if let [p] = params.as_slice() {
                if p.name.node == "__dot" && p.ty.is_none() {
                    if let Some(shorthand) = fmt_dot_shorthand(&body.node, indent) {
                        return shorthand;
                    }
                }
            }
            // The parameter list is always parenthesised — the parser has no
            // bare `x => ...` form (BACKLOG item 404), even for one parameter.
            let ps: Vec<String> = params.iter().map(fmt_lambda_param).collect();
            let b = fmt_expr(&body.node, indent);
            format!("({}) => {}", ps.join(", "), b)
        }

        Expr::List { elements, .. } => {
            let elems: Vec<String> = elements.iter().map(|e| fmt_expr(&e.node, indent)).collect();
            group(&elems, "[", "]", ", ", indent, false)
        }

        Expr::Tuple { elements, .. } => {
            let elems: Vec<String> = elements.iter().map(|e| fmt_expr(&e.node, indent)).collect();
            group(&elems, "(", ")", ", ", indent, false)
        }

        // Four real shapes share this one AST node (BACKLOG item 176 —
        // the previous version here ignored `ty_name` entirely and used a
        // bare `with` keyword that isn't valid expression syntax anywhere
        // in this language, so reformatting silently corrupted every one
        // of them into unparseable output):
        //   - `ty_name: Some, base: None`       -> `TypeName { field: value }`
        //   - `ty_name: Some, base: Some`       -> `TypeName { ..base, field: value }`
        //     (the real spread grammar, `parse_expr.rs`'s `parse_record_body`)
        //   - `ty_name: None, base: Some`       -> `base.with(field: value)`
        //     (item 151's postfix `.with(...)` desugar — never has a
        //     syntactic type name, always resolved from `base`'s own type)
        //   - `ty_name: None, base: None`       -> no real source form
        //     exists for a bare, unnamed record literal in this language
        //     (item 145) — formatted permissively rather than panicking.
        Expr::Record { ty_name, base, fields, .. } => {
            let fs: Vec<String> = fields.iter()
                .map(|f| format!("{}: {}", f.name.node, fmt_expr(&f.value.node, indent + 1)))
                .collect();
            match (ty_name, base) {
                (Some(name), None) => {
                    let body = group(&fs, "{ ", " }", ", ", indent, true);
                    format!("{} {}", name, body)
                }
                (Some(name), Some(b)) => {
                    let mut parts = vec![format!("..{}", fmt_expr(&b.node, indent))];
                    parts.extend(fs);
                    let body = group(&parts, "{ ", " }", ", ", indent, true);
                    format!("{} {}", name, body)
                }
                (None, Some(b)) => {
                    format!("{}.with({})", fmt_expr(&b.node, indent), fs.join(", "))
                }
                (None, None) => group(&fs, "{ ", " }", ", ", indent, true),
            }
        }

        Expr::Try      { expr, .. }  => format!("{}?", fmt_receiver(&expr.node, indent)),
        Expr::Await    { expr, .. }  => format!("await {}", fmt_receiver(&expr.node, indent)),
        Expr::Spawn    { expr, .. }  => format!("spawn {}", fmt_receiver(&expr.node, indent)),
        Expr::Unsafe   { body, .. }  => format!("unsafe {}", fmt_braced_body(&body.node, indent)),
        Expr::Transaction { body, .. } => format!("db.transaction {}", fmt_braced_body(&body.node, indent)),

        Expr::Guard { cond, else_expr, .. } => {
            format!("guard {} else {}", fmt_expr(&cond.node, indent), fmt_expr(&else_expr.node, indent))
        }

        Expr::Require { expr, error, .. } => {
            // The parser reads the checked expression as a postfix chain, then `(error)`.
            format!("require {} ({})", fmt_receiver(&expr.node, indent), fmt_expr(&error.node, indent))
        }

        Expr::Parallel { tasks, timeout, .. } => {
            let task_strs: Vec<String> = tasks.iter()
                .map(|t| format!("{}{}", ind(indent + 1), fmt_expr(&t.node, indent + 1)))
                .collect();
            let body = task_strs.join(",\n");
            let timeout_str = timeout.as_ref()
                .map(|t| format!("\n{}timeout: {}", ind(indent + 1), fmt_expr(&t.node, indent + 1)))
                .unwrap_or_default();
            format!("parallel {{\n{}{}\n{}}}", body, timeout_str, ind(indent))
        }

        Expr::WithTimeout { duration, body, .. } => {
            format!("withTimeout({}) {}", fmt_expr(&duration.node, indent), fmt_expr(&body.node, indent))
        }

        Expr::Ascribe { expr, ty, .. } =>
            format!("{}: {}", fmt_expr(&expr.node, indent), fmt_type(&ty.node, indent)),

        Expr::For { binding, iter, body, .. } => {
            format!("for {} in {} {}",
                binding.node,
                fmt_expr(&iter.node, indent),
                fmt_braced_body(&body.node, indent))
        }

        Expr::While { cond, body, .. } => {
            format!("while {} {}", fmt_expr(&cond.node, indent), fmt_braced_body(&body.node, indent))
        }
        Expr::Age { expr, .. } => {
            format!("{}.age", fmt_receiver(&expr.node, indent))
        }
        Expr::ExpectAssertion { actual, matcher, .. } => {
            let base = fmt_receiver(&actual.node, indent);
            match matcher {
                ExpectMatcher::ToBe(y)  => format!("{}.toBe({})", base, fmt_expr(&y.node, indent)),
                ExpectMatcher::ToBeTrue  => format!("{}.toBeTrue()", base),
                ExpectMatcher::ToBeFalse => format!("{}.toBeFalse()", base),
                ExpectMatcher::ToBeSome  => format!("{}.toBeSome()", base),
                ExpectMatcher::ToBeNone  => format!("{}.toBeNone()", base),
                ExpectMatcher::ToBeOk    => format!("{}.toBeOk()", base),
                ExpectMatcher::ToBeErr   => format!("{}.toBeErr()", base),
            }
        }
    }
}

/// Print the body of a `for`/`while`/`unsafe`/`db.transaction`/`defer`, which
/// the parser reads as an `Expr::Block` — braces included. Wrapping it in
/// braces again produced `{ { ... } }` and added another layer on every
/// `certo fmt` run, so a file grew each time it was formatted.
pub(crate) fn fmt_braced_body(body: &Expr, indent: usize) -> String {
    match body {
        Expr::Block { .. } => fmt_expr(body, indent),
        other => format!("{{\n{}{}\n{}}}", ind(indent + 1), fmt_expr(other, indent + 1), ind(indent)),
    }
}

pub fn fmt_stmt(stmt: &Stmt, indent: usize) -> String {
    match stmt {
        Stmt::Val { pattern, ty, value, .. } => {
            let pat = fmt_pat(&pattern.node, indent);
            let ty_str = ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
            // The annotation follows the pattern: `val n: Int = 1`. It used to be
            // printed before it (`val: Int n = 1`), which doesn't parse.
            format!("val {}{} = {}", pat, ty_str, fmt_expr(&value.node, indent))
        }
        Stmt::Var { name, ty, value, .. } => {
            let ty_str = ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
            format!("var {}{} = {}", name.node, ty_str, fmt_expr(&value.node, indent))
        }
        Stmt::Assign { target, value, .. } =>
            format!("{} = {}", target.node, fmt_expr(&value.node, indent)),
        Stmt::Defer { body, .. } =>
            format!("defer {}", fmt_braced_body(&body.node, indent)),
        Stmt::Expr { expr, .. } =>
            fmt_expr(&expr.node, indent),
    }
}

pub fn fmt_pat(pat: &Pattern, indent: usize) -> String {
    match pat {
        Pattern::Wildcard { .. }           => "_".to_string(),
        Pattern::Ident    { name, .. }     => name.node.clone(),
        Pattern::Constructor { path, fields, .. } => {
            let p = path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".");
            if fields.is_empty() { return p; }
            let fs: Vec<String> = fields.iter().map(|f| fmt_pat(&f.node, indent)).collect();
            format!("{}({})", p, fs.join(", "))
        }
        Pattern::Record { path, fields, rest, .. } => {
            let path_str = path.as_ref()
                .map(|p| format!("{} ", p.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".")))
                .unwrap_or_default();
            let mut fs: Vec<String> = fields.iter().map(|f| {
                if let Some(p) = &f.pattern {
                    format!("{}: {}", f.name.node, fmt_pat(&p.node, indent))
                } else {
                    f.name.node.clone()
                }
            }).collect();
            if *rest { fs.push("..".to_string()); }
            format!("{}{{ {} }}", path_str, fs.join(", "))
        }
        Pattern::Tuple { elements, .. } => {
            let es: Vec<String> = elements.iter().map(|e| fmt_pat(&e.node, indent)).collect();
            format!("({})", es.join(", "))
        }
        Pattern::List { head, tail, .. } => {
            let mut items: Vec<String> = head.iter().map(|h| fmt_pat(&h.node, indent)).collect();
            if let Some(t) = tail { items.push(format!("...{}", fmt_pat(&t.node, indent))); }
            format!("[{}]", items.join(", "))
        }
        Pattern::Literal { value, .. } => fmt_lit_pat(value),
        Pattern::Guard { pattern, guard, .. } =>
            format!("{} if {}", fmt_pat(&pattern.node, indent), fmt_expr(&guard.node, indent)),
        Pattern::As { pattern, name, .. } =>
            format!("{} as {}", fmt_pat(&pattern.node, indent), name.node),
        Pattern::Or { left, right, .. } =>
            format!("{} | {}", fmt_pat(&left.node, indent), fmt_pat(&right.node, indent)),
    }
}

fn fmt_arm(arm: &MatchArm, indent: usize) -> String {
    let pat = fmt_pat(&arm.pattern.node, indent);
    let guard = arm.guard.as_ref()
        .map(|g| format!(" if {}", fmt_expr(&g.node, indent)))
        .unwrap_or_default();
    let body = fmt_expr(&arm.body.node, indent);
    format!("{}{}{} => {}", ind(indent), pat, guard, body)
}

fn fmt_arg(arg: &Arg, indent: usize) -> String {
    if let Some(label) = &arg.label {
        format!("{}: {}", label.node, fmt_expr(&arg.value.node, indent))
    } else {
        fmt_expr(&arg.value.node, indent)
    }
}

fn fmt_lambda_param(p: &LambdaParam) -> String {
    if let Some(ty) = &p.ty {
        format!("{}: {}", p.name.node, fmt_type(&ty.node, 0))
    } else {
        p.name.node.clone()
    }
}

/// `true` for a bare reference to the leading-dot shorthand's own
/// synthesized receiver (`__dot`) — see `Expr::Lambda`'s own doc comment
/// above for why this exact name is safe to key off of.
fn is_dot_shorthand_receiver(expr: &Expr) -> bool {
    matches!(expr, Expr::Path { path, .. } if path.segments.len() == 1 && path.segments[0].node == "__dot")
}

/// Prints `body` back in its original leading-dot shorthand spelling
/// (`.field` / `.method(args)`) when it's exactly the shape the parser's
/// own desugar produces — `None` for anything else, so the caller falls
/// back to printing an ordinary lambda.
fn fmt_dot_shorthand(body: &Expr, indent: usize) -> Option<String> {
    match body {
        Expr::Field { expr, field, .. } if is_dot_shorthand_receiver(&expr.node) =>
            Some(format!(".{}", field.node)),
        Expr::App { func, args, .. } => {
            let Expr::Field { expr, field, .. } = &func.node else { return None };
            if !is_dot_shorthand_receiver(&expr.node) { return None; }
            let formatted_args: Vec<String> = args.iter().map(|a| fmt_arg(a, indent)).collect();
            let args_str = group(&formatted_args, "(", ")", ", ", indent, false);
            Some(format!(".{}{}", field.node, args_str))
        }
        _ => None,
    }
}

fn fmt_lit(lit: &Lit) -> String {
    match lit {
        Lit::Int(n)        => n.to_string(),
        Lit::Float(f)      => {
            let s = format!("{}", f);
            if s.contains('.') { s } else { format!("{}.0", s) }
        }
        Lit::Decimal(s)    => format!("d\"{}\"", s),
        Lit::Bool(b)       => if *b { "true".into() } else { "false".into() },
        Lit::String(s)     => format!("\"{}\"", escape_str(s)),
        Lit::FString(parts) => {
            // Literal parts hold the unescaped text (the parser processes
            // escapes, BACKLOG item 431), so they are escaped again here like
            // any string. Interpolation is plain `{expr}`; a `$` before the
            // brace would become literal text (BACKLOG item 404).
            let inner: String = parts.iter().map(|p| match p {
                FStringPart::Literal(s) => escape_str(s),
                FStringPart::Interpolated(e) => format!("{{{}}}", fmt_expr(&e.node, 0)),
            }).collect();
            format!("f\"{}\"", inner)
        }
        Lit::Uuid(s)       => format!("uuid\"{}\"", s),
        Lit::Unit           => "()".into(),
    }
}

fn fmt_lit_pat(lit: &certo_ast::pattern::LitPat) -> String {
    use certo_ast::pattern::LitPat;
    match lit {
        LitPat::Int(n)    => n.to_string(),
        LitPat::Float(f)  => format!("{}", f),
        LitPat::Bool(b)   => if *b { "true".into() } else { "false".into() },
        LitPat::String(s) => format!("\"{}\"", escape_str(s)),
        LitPat::Unit      => "()".into(),
    }
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"',  "\\\"")
     .replace('\n', "\\n")
     .replace('\r', "\\r")
     .replace('\t', "\\t")
}
