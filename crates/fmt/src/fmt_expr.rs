use certo_ast::expr::{Expr, Stmt, Lit, FStringPart, UnOp, Arg, MatchArm, LambdaParam};
use certo_ast::pattern::Pattern;
use crate::printer::{ind, group, binop_str};
use crate::fmt_type::fmt_type;

pub fn fmt_expr(expr: &Expr, indent: usize) -> String {
    match expr {
        Expr::Lit { value, .. } => fmt_lit(value),

        Expr::Path { path, .. } =>
            path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join("."),

        Expr::App { func, args, .. } => {
            let f = fmt_expr(&func.node, indent);
            let formatted_args: Vec<String> = args.iter().map(|a| fmt_arg(a, indent)).collect();
            let args_str = group(&formatted_args, "(", ")", ", ", indent, false);
            format!("{}{}", f, args_str)
        }

        Expr::Pipe { left, right, .. } => {
            let l = fmt_expr(&left.node, indent);
            let r = fmt_expr(&right.node, indent);
            let inline = format!("{} |> {}", l, r);
            if inline.len() <= 80 {
                inline
            } else {
                format!("{}\n{}|> {}", l, ind(indent + 1), r)
            }
        }

        Expr::BinOp { op, left, right, .. } => {
            let l = fmt_expr(&left.node, indent);
            let r = fmt_expr(&right.node, indent);
            // Wrap in parens if the sub-expression is itself a binop with lower precedence.
            format!("{} {} {}", l, binop_str(op), r)
        }

        Expr::UnOp { op, expr, .. } => {
            let e = fmt_expr(&expr.node, indent);
            match op {
                UnOp::Neg => format!("-{}", e),
                UnOp::Not => format!("not {}", e),
            }
        }

        Expr::Field { expr, field, .. } =>
            format!("{}.{}", fmt_expr(&expr.node, indent), field.node),

        Expr::SafeField { expr, field, .. } =>
            format!("{}?.{}", fmt_expr(&expr.node, indent), field.node),

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
            let ps: Vec<String> = params.iter().map(fmt_lambda_param).collect();
            let param_str = if ps.len() == 1 { ps[0].clone() } else { format!("({})", ps.join(", ")) };
            let b = fmt_expr(&body.node, indent);
            format!("{} => {}", param_str, b)
        }

        Expr::List { elements, .. } => {
            let elems: Vec<String> = elements.iter().map(|e| fmt_expr(&e.node, indent)).collect();
            group(&elems, "[", "]", ", ", indent, false)
        }

        Expr::Tuple { elements, .. } => {
            let elems: Vec<String> = elements.iter().map(|e| fmt_expr(&e.node, indent)).collect();
            group(&elems, "(", ")", ", ", indent, false)
        }

        Expr::Record { base, fields, .. } => {
            let fs: Vec<String> = fields.iter()
                .map(|f| format!("{}: {}", f.name.node, fmt_expr(&f.value.node, indent + 1)))
                .collect();
            let body = group(&fs, "{ ", " }", ", ", indent, true);
            if let Some(b) = base {
                format!("{} with {}", fmt_expr(&b.node, indent), body)
            } else {
                body
            }
        }

        Expr::Try      { expr, .. }  => format!("{}?", fmt_expr(&expr.node, indent)),
        Expr::Await    { expr, .. }  => format!("await {}", fmt_expr(&expr.node, indent)),
        Expr::Unsafe   { body, .. }  => format!("unsafe {{\n{}{}\n{}}}", ind(indent + 1), fmt_expr(&body.node, indent + 1), ind(indent)),
        Expr::Transaction { body, .. } => format!("db.transaction {{\n{}{}\n{}}}", ind(indent + 1), fmt_expr(&body.node, indent + 1), ind(indent)),

        Expr::Guard { cond, else_expr, .. } => {
            format!("guard {} else {}", fmt_expr(&cond.node, indent), fmt_expr(&else_expr.node, indent))
        }

        Expr::Require { expr, error, .. } => {
            format!("require {} ({})", fmt_expr(&expr.node, indent), fmt_expr(&error.node, indent))
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

        Expr::Ascribe { expr, ty, .. } =>
            format!("{}: {}", fmt_expr(&expr.node, indent), fmt_type(&ty.node, indent)),
    }
}

pub fn fmt_stmt(stmt: &Stmt, indent: usize) -> String {
    match stmt {
        Stmt::Val { pattern, ty, value, .. } => {
            let pat = fmt_pat(&pattern.node, indent);
            let ty_str = ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
            format!("val{} {} = {}", ty_str, pat, fmt_expr(&value.node, indent))
        }
        Stmt::Var { name, ty, value, .. } => {
            let ty_str = ty.as_ref().map(|t| format!(": {}", fmt_type(&t.node, indent))).unwrap_or_default();
            format!("var {}{} = {}", name.node, ty_str, fmt_expr(&value.node, indent))
        }
        Stmt::Assign { target, value, .. } =>
            format!("{} = {}", target.node, fmt_expr(&value.node, indent)),
        Stmt::Defer { body, .. } =>
            format!("defer {{\n{}{}\n{}}}", ind(indent + 1), fmt_expr(&body.node, indent + 1), ind(indent)),
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
            let inner: String = parts.iter().map(|p| match p {
                FStringPart::Literal(s) => escape_str(s),
                FStringPart::Interpolated(e) => format!("${{{}}}", fmt_expr(&e.node, 0)),
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
