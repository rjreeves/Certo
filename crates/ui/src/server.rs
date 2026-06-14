//! Generate a complete runnable Certo HTTP server from `view` and `form` declarations.
//!
//! # Output shape
//!
//! ```text
//! certo-ui fireworks_ui.cto          →  server.cto
//! certo build server.cto -o app.exe
//! DATABASE_URL=postgres://... app.exe 8080
//! ```
//!
//! Each `view` becomes a GET route that queries the DB and renders an HTML table.
//! Each `form` becomes a GET route (empty form) and a POST route (INSERT to DB).

use std::fmt::Write as FmtWrite;
use certo_ast::decl::{Decl, ViewDecl, FormDecl};
use certo_ast::expr::{Expr, Stmt};
use certo_ast::module::Module;
use certo_ast::span::S;
use crate::error::UiError;

/// Entry point — compile all `view`/`form` decls to a single `server.cto` string.
pub fn emit_server(module: &Module) -> Result<String, UiError> {
    let mod_name = module.path.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or("App");

    let mut views: Vec<&ViewDecl> = vec![];
    let mut forms: Vec<&FormDecl> = vec![];

    for sd in &module.decls {
        match &sd.node {
            Decl::View(v) => views.push(v),
            Decl::Form(f) => forms.push(f),
            _ => {}
        }
    }

    if views.is_empty() && forms.is_empty() {
        return Err(UiError::NoDeclarations);
    }

    let mut out = String::new();

    emit_header(&mod_name, &mut out);
    emit_shared_helpers(&mut out);
    emit_router(&views, &forms, &mut out);

    for v in &views {
        emit_view_handler(v, &forms, &mut out);
    }
    for f in &forms {
        emit_form_get_handler(f, &mut out);
        emit_form_post_handler(f, &mut out);
    }

    emit_main(&mut out);

    Ok(out)
}

// ── Module header ─────────────────────────────────────────────────────────────

fn emit_header(mod_name: &str, out: &mut String) {
    writeln!(out, "module {}Server", mod_name).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "import Stdlib.Core").unwrap();
    writeln!(out, "import Stdlib.Http").unwrap();
    writeln!(out, "import Stdlib.Db").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn dbUrl(): Text = getEnv(\"DATABASE_URL\") ?? \"host=localhost dbname=postgres user=postgres\"").unwrap();
    writeln!(out).unwrap();
}

// ── Shared helpers emitted once into every generated server ───────────────────

fn emit_shared_helpers(out: &mut String) {
    writeln!(out, "// ── CSS + page shell ─────────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn htmlCss(): Text =").unwrap();
    writeln!(out, "    \"<style>\" ++").unwrap();
    writeln!(out, "    \"body{{font-family:sans-serif;margin:2rem;color:#222;}}\" ++").unwrap();
    writeln!(out, "    \"h1{{font-size:1.5rem;margin-bottom:1rem;}}\" ++").unwrap();
    writeln!(out, "    \"nav{{margin-bottom:1.5rem;}}\" ++").unwrap();
    writeln!(out, "    \"nav a{{margin-right:1rem;color:#0066cc;text-decoration:none;}}\" ++").unwrap();
    writeln!(out, "    \"table{{border-collapse:collapse;width:100%;margin-top:0.5rem;}}\" ++").unwrap();
    writeln!(out, "    \"th,td{{border:1px solid #ccc;padding:6px 12px;text-align:left;font-size:0.9rem;}}\" ++").unwrap();
    writeln!(out, "    \"th{{background:#f0f0f0;font-weight:600;}}\" ++").unwrap();
    writeln!(out, "    \"tr:nth-child(even) td{{background:#fafafa;}}\" ++").unwrap();
    writeln!(out, "    \".field{{margin-bottom:1rem;}}\" ++").unwrap();
    writeln!(out, "    \"label{{display:block;font-weight:bold;margin-bottom:0.25rem;}}\" ++").unwrap();
    writeln!(out, "    \"input,select,textarea{{padding:0.4rem;border:1px solid #ccc;border-radius:4px;width:100%;max-width:420px;box-sizing:border-box;}}\" ++").unwrap();
    writeln!(out, "    \"button{{padding:0.5rem 1.25rem;background:#0066cc;color:#fff;border:none;border-radius:4px;cursor:pointer;margin-top:0.5rem;}}\" ++").unwrap();
    writeln!(out, "    \"a.btn{{display:inline-block;padding:0.4rem 1rem;background:#0066cc;color:#fff;border-radius:4px;text-decoration:none;font-size:0.9rem;}}\" ++").unwrap();
    writeln!(out, "    \"</style>\"").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "fn htmlPage(title: Text, body: Text): Text =").unwrap();
    writeln!(out, "    \"<!DOCTYPE html><html lang=\\\"en\\\"><head>\" ++").unwrap();
    writeln!(out, "    \"<meta charset=\\\"UTF-8\\\">\" ++").unwrap();
    writeln!(out, "    \"<meta name=\\\"viewport\\\" content=\\\"width=device-width,initial-scale=1\\\">\" ++").unwrap();
    writeln!(out, "    \"<title>\" ++ title ++ \"</title>\" ++").unwrap();
    writeln!(out, "    htmlCss() ++").unwrap();
    writeln!(out, "    \"</head><body>\" ++ body ++ \"</body></html>\"").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "// ── HTML table helpers ───────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn htmlTh(cols: List<Text>, i: Int, n: Int, acc: Text): Text =").unwrap();
    writeln!(out, "    if i >= n then acc").unwrap();
    writeln!(out, "    else htmlTh(cols, i + 1, n, acc ++ \"<th>\" ++ List.getOrPanic(cols, i) ++ \"</th>\")").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn htmlTd(row: List<Text?>, i: Int, n: Int, acc: Text): Text =").unwrap();
    writeln!(out, "    if i >= n then acc").unwrap();
    writeln!(out, "    else htmlTd(row, i + 1, n, acc ++ \"<td>\" ++ (List.getOrPanic(row, i) ?? \"\") ++ \"</td>\")").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn htmlTr(rows: List<List<Text?>>, i: Int, n: Int, acc: Text): Text =").unwrap();
    writeln!(out, "    if i >= n then acc").unwrap();
    writeln!(out, "    else {{").unwrap();
    writeln!(out, "        val row = List.getOrPanic(rows, i)").unwrap();
    writeln!(out, "        htmlTr(rows, i + 1, n, acc ++ \"<tr>\" ++ htmlTd(row, 0, List.len(row), \"\") ++ \"</tr>\")").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn htmlTable(cols: List<Text>, rows: List<List<Text?>>): Text =").unwrap();
    writeln!(out, "    \"<table><thead><tr>\" ++ htmlTh(cols, 0, List.len(cols), \"\") ++").unwrap();
    writeln!(out, "    \"</tr></thead><tbody>\" ++ htmlTr(rows, 0, List.len(rows), \"\") ++ \"</tbody></table>\"").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "// ── URL-encoded form body parsing ────────────────────────────────────────────").unwrap();
    writeln!(out, "fn formField(body: Text, name: Text): Text =").unwrap();
    writeln!(out, "    formFieldAt(Text.split(body, \"&\"), name, 0)").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "fn formFieldAt(parts: List<Text>, name: Text, i: Int): Text =").unwrap();
    writeln!(out, "    if i >= List.len(parts) then \"\"").unwrap();
    writeln!(out, "    else {{").unwrap();
    writeln!(out, "        val kv = Text.split(List.getOrPanic(parts, i), \"=\")").unwrap();
    writeln!(out, "        if List.len(kv) >= 2 and Text.eq(List.getOrPanic(kv, 0), name)").unwrap();
    writeln!(out, "        then List.getOrPanic(kv, 1)").unwrap();
    writeln!(out, "        else formFieldAt(parts, name, i + 1)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

// ── Router ────────────────────────────────────────────────────────────────────

fn emit_router(views: &[&ViewDecl], forms: &[&FormDecl], out: &mut String) {
    writeln!(out, "// ── Router ───────────────────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn handler(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val path   = HttpRequest.path(req)").unwrap();
    writeln!(out, "    val method = HttpRequest.method(req)").unwrap();

    let mut first = true;

    for v in views {
        let slug    = slugify(&v.name.node);
        let fn_name = format!("handle{}", v.name.node);
        let kw = if first { "    if" } else { "    else if" };
        writeln!(out, "{} path == \"/{slug}\" then {fn_name}(req)", kw).unwrap();
        first = false;
    }

    for f in forms {
        let slug    = slugify(&f.name.node);
        let fn_get  = format!("handle{}Get", f.name.node);
        let fn_post = format!("handle{}Post", f.name.node);
        let kw = if first { "    if" } else { "    else if" };
        writeln!(out, "{} path == \"/{slug}\" then", kw).unwrap();
        writeln!(out, "        if method == \"POST\" then {fn_post}(req)").unwrap();
        writeln!(out, "        else {fn_get}(req)").unwrap();
        first = false;
    }

    if first {
        writeln!(out, "    Http.notFound(\"No routes defined\")").unwrap();
    } else {
        writeln!(out, "    else Http.notFound(f\"No route for {{method}} {{path}}\")").unwrap();
    }

    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── View handler ──────────────────────────────────────────────────────────────

fn emit_view_handler(v: &ViewDecl, forms: &[&FormDecl], out: &mut String) {
    let name    = &v.name.node;
    let fn_name = format!("handle{}", name);
    let title   = to_title(name);

    writeln!(out, "// ── {} ─────────────────────────────────────────────────────────────────────", name).unwrap();

    // Look for For(items, item) to find the entity/table
    let for_entity = find_for_entity(&v.layout.node);

    if let Some(ref entity) = for_entity {
        let table = entity.to_lowercase();
        let sql   = format!("SELECT * FROM {} ORDER BY 1", table);

        // Nav link to the "create" form for this entity, if one exists
        let create_slug = forms.iter()
            .find(|f| {
                let fn_lower = f.name.node.to_lowercase();
                fn_lower.starts_with("create") && fn_lower.contains(&entity.to_lowercase())
            })
            .map(|f| slugify(&f.name.node));

        let nav_html = match create_slug {
            Some(cs) => format!(
                "\"<nav><a href=\\\"/{cs}\\\" class=\\\"btn\\\">New {}</a></nav>\" ++",
                to_title(entity)
            ),
            None => String::new(),
        };

        writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();
        writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
        writeln!(out, "    val sql  = \"{sql}\"").unwrap();
        writeln!(out, "    val cols = dbColumns(conn, sql)").unwrap();
        writeln!(out, "    val rows = dbQuery(conn, sql, List.empty)").unwrap();
        writeln!(out, "    dbClose(conn)").unwrap();
        writeln!(out, "    val body = \"<h1>{title}</h1>\" ++").unwrap();
        writeln!(out, "        {nav_html}").unwrap();
        writeln!(out, "        htmlTable(cols, rows)").unwrap();
        writeln!(out, "    Http.ok(htmlPage(\"{title}\", body), \"text/html\")").unwrap();
        writeln!(out, "}}").unwrap();
    } else {
        // Static view — no data binding, render layout as fixed HTML
        let body_html = {
            use crate::layout::layout_to_html;
            let layout_expr = extract_layout_expr(&v.layout.node).unwrap_or(&v.layout.node);
            let raw = layout_to_html(layout_expr, 0);
            // Strip HTML comments for embedding
            raw.lines()
               .filter(|l| !l.trim_start().starts_with("<!--"))
               .collect::<Vec<_>>()
               .join("")
        };
        let escaped = escape_certo_str(&format!("<h1>{title}</h1>{body_html}"));

        writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse =").unwrap();
        writeln!(out, "    Http.ok(htmlPage(\"{title}\", \"{escaped}\"), \"text/html\")").unwrap();
    }

    writeln!(out).unwrap();
}

// ── Form GET handler ──────────────────────────────────────────────────────────

fn emit_form_get_handler(f: &FormDecl, out: &mut String) {
    let name    = &f.name.node;
    let fn_name = format!("handle{}Get", name);
    let slug    = slugify(name);
    let title   = to_title(name);

    // Infer where the list lives for a "Back" link
    let target = f.target.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or(name.as_str());
    let list_slug = slugify(&format!("{}List", target));

    writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val body =").unwrap();
    writeln!(out, "        \"<h1>{title}</h1>\" ++").unwrap();
    writeln!(out, "        \"<nav><a href=\\\"/{list_slug}\\\">← Back to list</a></nav>\" ++").unwrap();
    writeln!(out, "        \"<form method=\\\"POST\\\" action=\\\"/{slug}\\\">\" ++").unwrap();

    for field in &f.fields {
        let fname = &field.name.node;
        let label = field.label.as_deref().unwrap_or(fname.as_str());
        let itype = infer_input_type(fname, &field.field_type);
        writeln!(out,
            "        \"<div class=\\\"field\\\"><label for=\\\"{fname}\\\">{label}</label>\" ++").unwrap();
        writeln!(out,
            "        \"<input id=\\\"{fname}\\\" name=\\\"{fname}\\\" type=\\\"{itype}\\\" required></div>\" ++").unwrap();
    }

    writeln!(out, "        \"<button type=\\\"submit\\\">Submit</button>\" ++").unwrap();
    writeln!(out, "        \"</form>\"").unwrap();
    writeln!(out, "    Http.ok(htmlPage(\"{title}\", body), \"text/html\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── Form POST handler ─────────────────────────────────────────────────────────

fn emit_form_post_handler(f: &FormDecl, out: &mut String) {
    let name    = &f.name.node;
    let fn_name = format!("handle{}Post", name);
    let target  = f.target.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or(name.as_str());
    let table      = camel_to_snake(target);
    let list_slug  = slugify(&format!("{}List", target));

    // Column names (snake_case of field names) and SQL placeholders
    let cols: Vec<String> = f.fields.iter()
        .map(|field| camel_to_snake(&field.name.node))
        .collect();
    let col_list = cols.join(", ");
    let placeholders: Vec<String> = (1..=cols.len())
        .map(|i| format!("${}", i))
        .collect();
    let ph_list = placeholders.join(", ");
    let sql = format!("INSERT INTO {table} ({col_list}) VALUES ({ph_list})");

    writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val body = HttpRequest.body(req)").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();

    // Extract each form field from the body
    for field in &f.fields {
        let fname = &field.name.node;
        writeln!(out, "    val {fname} = formField(body, \"{fname}\")").unwrap();
    }

    // Build params list: val _p0 = List.empty; val _p1 = List.push(_p0, f0); ...
    writeln!(out, "    val _p0 = List.empty").unwrap();
    for (i, field) in f.fields.iter().enumerate() {
        let fname = &field.name.node;
        writeln!(out, "    val _p{} = List.push(_p{}, {})", i + 1, i, fname).unwrap();
    }
    let last_p = format!("_p{}", f.fields.len());

    writeln!(out, "    val _rows = dbExec(conn, \"{sql}\", {last_p})").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    writeln!(out, "    val msg = if _rows > 0 then \"Record saved.\" else \"Save failed — check your input.\"").unwrap();
    writeln!(out, "    val pg  = \"<h2>\" ++ msg ++ \"</h2><p><a href=\\\"/{list_slug}\\\">← Back to list</a></p>\"").unwrap();
    writeln!(out, "    Http.ok(htmlPage(\"Done\", pg), \"text/html\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn emit_main(out: &mut String) {
    writeln!(out, "fn main(): Unit [io] = {{").unwrap();
    writeln!(out, "    println(\"Serving on http://localhost:3000/\")").unwrap();
    writeln!(out, "    Http.serve(3000, handler)").unwrap();
    writeln!(out, "}}").unwrap();
}

// ── Layout expression extraction ──────────────────────────────────────────────

/// Pull the RHS of `layout = <expr>` from a view body block.
pub fn extract_layout_expr(block: &Expr) -> Option<&Expr> {
    if let Expr::Block { stmts, .. } = block {
        for stmt in stmts {
            if let Stmt::Assign { target, value, .. } = stmt {
                if target.node == "layout" {
                    return Some(&value.node);
                }
            }
        }
    }
    None
}

/// Recursively find the second positional arg of the first `For(items, item)` call.
fn find_for_entity(expr: &Expr) -> Option<String> {
    match expr {
        Expr::App { func, args, .. } => {
            if call_name(func).as_deref() == Some("For") {
                let positional: Vec<_> = args.iter().filter(|a| a.label.is_none()).collect();
                if let Some(arg) = positional.get(1) {
                    if let Expr::Path { path, .. } = &arg.value.node {
                        if let Some(seg) = path.segments.last() {
                            return Some(seg.node.clone());
                        }
                    }
                }
            }
            for arg in args {
                if let Some(e) = find_for_entity(&arg.value.node) {
                    return Some(e);
                }
            }
            None
        }
        Expr::List { elements, .. } => {
            elements.iter().find_map(|e| find_for_entity(&e.node))
        }
        Expr::Block { stmts, .. } => {
            stmts.iter().find_map(|stmt| {
                if let Stmt::Assign { value, .. } = stmt {
                    find_for_entity(&value.node)
                } else {
                    None
                }
            })
        }
        _ => None,
    }
}

fn call_name(func: &S<Expr>) -> Option<String> {
    if let Expr::Path { path, .. } = &func.node {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

// ── String utilities ──────────────────────────────────────────────────────────

pub fn slugify(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 { s.push('-'); }
        s.push(c.to_lowercase().next().unwrap());
    }
    s
}

pub fn camel_to_snake(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 { s.push('_'); }
        s.push(c.to_lowercase().next().unwrap());
    }
    s
}

fn to_title(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 { s.push(' '); }
        s.push(c);
    }
    s
}

/// Escape a string for embedding inside a Certo double-quoted string literal.
fn escape_certo_str(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"', "\\\"")
     .replace('\n', "")
     .replace('\r', "")
}

fn infer_input_type(name: &str, _ft: &Option<certo_ast::span::S<certo_ast::expr::Expr>>) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("email")                          { return "email"; }
    if lower.contains("password")                       { return "password"; }
    if lower.contains("phone") || lower.contains("tel") { return "tel"; }
    if lower.contains("date")                           { return "date"; }
    if lower.contains("url") || lower.contains("website") { return "url"; }
    "text"
}
