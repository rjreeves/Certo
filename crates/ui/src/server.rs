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
use certo_ast::decl::{Decl, ViewDecl, FormDecl, FormField, TypeBody, RecordFieldDef, UiGenerateDecl};
use certo_ast::expr::{Expr, Stmt};
use certo_ast::module::Module;
use certo_ast::span::S;
use certo_ast::types::TypeExpr;
use crate::error::UiError;

/// Entry point — compile all `view`/`form` decls (including any lowered
/// from `@ui.generate`, BACKLOG item 87) to a single `server.cto` string.
pub fn emit_server(module: &Module) -> Result<String, UiError> {
    let mod_name = module.path.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or("App");

    let mut views: Vec<&ViewDecl> = vec![];
    let mut forms: Vec<&FormDecl> = vec![];
    // Owned storage for view/form decls lowered from `@ui.generate` — must
    // outlive the loop below so `views`/`forms` can hold references into it.
    let mut generated: Vec<S<Decl>> = vec![];

    for sd in &module.decls {
        match &sd.node {
            Decl::View(v) => views.push(v),
            Decl::Form(f) => forms.push(f),
            Decl::UiGenerate(g) => {
                generated.extend(lower_ui_generate(g, module)?);
            }
            _ => {}
        }
    }
    for sd in &generated {
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
    emit_pk_col_index_helper(&mut out);
    emit_col_value_helper(&mut out);
    for v in &views {
        emit_row_actions_for_view(v, &forms, &views, &mut out);
    }
    emit_router(&views, &forms, &mut out);

    for v in &views {
        emit_view_handler(v, &forms, &views, &mut out);
    }
    // BACKLOG item 88 (stage 3) — only wire the notify-on-write call when
    // some view actually has a `live val` to notify; a module with none
    // gets exactly the write-handler output it had before this item.
    let has_live = views.iter().any(|v| !v.live.is_empty());
    for f in &forms {
        emit_form_get_handler(f, module, &mut out);
        emit_form_post_handler(f, has_live, &mut out);
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
    writeln!(out, "    \".row-actions{{margin:2px 0;font-size:0.85rem;}}\" ++").unwrap();
    writeln!(out, "    \".row-actions a{{margin-right:0.75rem;color:#0066cc;}}\" ++").unwrap();
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
        writeln!(out, "        if Text.eq(method, \"POST\") then {fn_post}(req)").unwrap();
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

fn emit_view_handler(v: &ViewDecl, forms: &[&FormDecl], _all_views: &[&ViewDecl], out: &mut String) {
    let name    = &v.name.node;
    let fn_name = format!("handle{}", name);
    let title   = to_title(name);

    writeln!(out, "// ── {} ─────────────────────────────────────────────────────────────────────", name).unwrap();

    // BACKLOG item 88 (stage 3): a view with `live val` bindings gets a
    // tiny inline SSE client wired to the reserved `/__certo_live`
    // endpoint (`crates/stdlib/src/http.rs`'s `certo_http_serve`) —
    // `certo_live_notify()` (called by generated write handlers below)
    // broadcasts on every successful DB write, and any connected page
    // just reloads itself on that signal. Deliberately a whole-page
    // reload, not a per-binding DOM patch or a per-table-scoped signal:
    // this codegen path never used htmx client-side attributes at all
    // before this (unlike `crates/ui/src/form.rs`'s legacy `--html`
    // mode), and a plain `EventSource` needs no new script dependency —
    // adding htmx's own SSE extension just to reload the page it's
    // already on would be more machinery for the same result.
    let live_script = if v.live.is_empty() {
        String::new()
    } else {
        "<script>new EventSource('/__certo_live').onmessage=function(){location.reload();};</script>".to_string()
    };
    if !v.live.is_empty() {
        let bindings: Vec<&str> = v.live.iter()
            .filter_map(|vd| match &vd.pattern.node {
                certo_ast::pattern::Pattern::Ident { name, .. } => Some(name.node.as_str()),
                _ => None,
            })
            .collect();
        writeln!(out, "// live bindings wired to SSE auto-refresh via /__certo_live: {}", bindings.join(", ")).unwrap();
    }

    let for_entity = find_for_entity(&v.layout.node);

    if let Some(ref entity) = for_entity {
        let table      = entity.to_lowercase();
        let _pk_col     = v.pk.as_deref().map(camel_to_snake);
        let filter_col = v.filter_by.as_deref().map(camel_to_snake);

        // SQL — with optional WHERE for filter
        let sql = if let Some(ref fc) = filter_col {
            format!("SELECT * FROM {} WHERE {} = $1 ORDER BY 1", table, fc)
        } else {
            format!("SELECT * FROM {} ORDER BY 1", table)
        };

        // Create-form nav link
        let create_form = forms.iter().find(|f| {
            let fl = f.name.node.to_lowercase();
            fl.starts_with("create") && fl.contains(&entity.to_lowercase())
        });

        writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();
        writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();
        writeln!(out, "    val sql  = \"{sql}\"").unwrap();

        // If filtered, extract the param from query string
        if let Some(ref fc) = filter_col {
            writeln!(out, "    val _qry   = HttpRequest.query(req)").unwrap();
            writeln!(out, "    val _fval  = formField(_qry, \"{fc}\")").unwrap();
            writeln!(out, "    val _p0    = List.empty()").unwrap();
            writeln!(out, "    val _p1    = List.push(_p0, _fval)").unwrap();
            writeln!(out, "    val cols   = dbColumns(conn, \"SELECT * FROM {table} ORDER BY 1\")").unwrap();
            writeln!(out, "    val rows   = dbQuery(conn, sql, _p1)").unwrap();
        } else {
            writeln!(out, "    val cols   = dbColumns(conn, sql)").unwrap();
            writeln!(out, "    val rows   = dbQuery(conn, sql, List.empty())").unwrap();
        }
        writeln!(out, "    dbClose(conn)").unwrap();

        // Build nav bar
        let mut nav_parts: Vec<String> = vec![];
        if let Some(cf) = create_form {
            let cs = slugify(&cf.name.node);
            let et = to_title(entity);
            if let Some(ref fc) = filter_col {
                nav_parts.push(format!("\"<a href=\\\"/{cs}?{fc}=\\\" class=\\\"btn\\\">New {et}</a>\""));
            } else {
                nav_parts.push(format!("\"<a href=\\\"/{cs}\\\" class=\\\"btn\\\">New {et}</a>\""));
            }
        }
        let nav_html = if nav_parts.is_empty() {
            String::new()
        } else {
            format!("\"<nav>\" ++ {} ++ \"</nav>\" ++\n        ", nav_parts.join(" ++ "))
        };

        writeln!(out, "    val _tbl  = htmlTable(cols, rows)").unwrap();

        let live_suffix = if live_script.is_empty() {
            String::new()
        } else {
            format!(" ++ \"{}\"", escape_certo_str(&live_script))
        };

        if v.pk.is_some() {
            let ra_fn = format!("rowActions{name}");
            writeln!(out, "    val _actions = {ra_fn}(cols, rows, 0, \"\")").unwrap();
            writeln!(out, "    val body = \"<h1>{title}</h1>\" ++").unwrap();
            writeln!(out, "        {nav_html}\"\" ++").unwrap();
            writeln!(out, "        _tbl ++").unwrap();
            writeln!(out, "        \"<div class=\\\"actions-section\\\">\" ++ _actions ++ \"</div>\"{live_suffix}").unwrap();
        } else {
            writeln!(out, "    val body = \"<h1>{title}</h1>\" ++").unwrap();
            writeln!(out, "        {nav_html}\"\" ++").unwrap();
            writeln!(out, "        _tbl{live_suffix}").unwrap();
        }

        writeln!(out, "    Http.ok(htmlPage(\"{title}\", body), \"text/html\")").unwrap();
        writeln!(out, "}}").unwrap();
    } else {
        // Static view
        //
        // BACKLOG item 215 — this previously filtered out every line
        // starting with `<!--` before splicing `layout_to_html`'s output in,
        // silently discarding the diagnostic HTML comment `layout_to_html`
        // itself emits for any unrecognized layout primitive (`layout.rs`'s
        // own doc comment: "any unrecognised call emits an HTML comment so
        // the file stays valid") — an unsupported primitive vanished with
        // zero trace in the page this handler actually serves, unlike the
        // legacy `--html` codegen mode, which already keeps these comments.
        // No longer stripped, so the two modes behave consistently and the
        // gap stays visible (in view-source) instead of silently disappearing.
        let body_html = {
            use crate::layout::layout_to_html;
            let layout_expr = extract_layout_expr(&v.layout.node).unwrap_or(&v.layout.node);
            layout_to_html(layout_expr, 0)
        };
        let escaped = escape_certo_str(&format!("<h1>{title}</h1>{body_html}{live_script}"));
        writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse =").unwrap();
        writeln!(out, "    Http.ok(htmlPage(\"{title}\", \"{escaped}\"), \"text/html\")").unwrap();
    }

    writeln!(out).unwrap();
}

// ── pkColIndex helper (emitted once, shared by all views) ─────────────────────

fn emit_pk_col_index_helper(out: &mut String) {
    writeln!(out, "// ── Column index lookup ──────────────────────────────────────────────────────").unwrap();
    writeln!(out, "fn pkColIndex(cols: List<Text>, pk: Text, i: Int): Int =").unwrap();
    writeln!(out, "    if i >= List.len(cols) then -1").unwrap();
    writeln!(out, "    else if Text.eq(List.getOrPanic(cols, i), pk) then i").unwrap();
    writeln!(out, "    else pkColIndex(cols, pk, i + 1)").unwrap();
    writeln!(out).unwrap();
}

// ── Per-view row-action function ──────────────────────────────────────────────

/// Emit `fn rowActionsXxx(cols, rows, i, acc): Text` with hardcoded edit + child links.
fn emit_row_actions_for_view(v: &ViewDecl, forms: &[&FormDecl], all_views: &[&ViewDecl], out: &mut String) {
    let pk = match &v.pk { Some(p) => p, None => return };
    let name    = &v.name.node;
    let fn_name = format!("rowActions{}", name);
    let pk_col  = camel_to_snake(pk);

    // Edit form for this entity
    let entity = match find_for_entity(&v.layout.node) { Some(e) => e, None => return };
    let edit_form = forms.iter().find(|f| {
        let fl = f.name.node.to_lowercase();
        fl.starts_with("edit") && fl.contains(&entity.to_lowercase())
    });

    // Child views whose filter_by matches our pk
    let child_views: Vec<&&ViewDecl> = all_views.iter().filter(|cv| {
        cv.filter_by.as_deref().map(camel_to_snake).as_deref() == Some(pk_col.as_str())
    }).collect();

    writeln!(out, "fn {fn_name}(cols: List<Text>, rows: List<List<Text?>>, i: Int, acc: Text): Text =").unwrap();
    writeln!(out, "    if i >= List.len(rows) then acc").unwrap();
    writeln!(out, "    else {{").unwrap();
    writeln!(out, "        val row   = List.getOrPanic(rows, i)").unwrap();
    writeln!(out, "        val pkIdx = pkColIndex(cols, \"{pk_col}\", 0)").unwrap();
    writeln!(out, "        val pkVal = if pkIdx >= 0 then (List.getOrPanic(row, pkIdx) ?? \"\") else \"\"").unwrap();

    // Build URL vals and links expression.
    // Each URL is a separate val so pkVal is concatenated as an operator, not
    // embedded as literal text inside a string.
    let mut url_vals: Vec<String>  = vec![];
    let mut link_parts: Vec<String> = vec![];

    if let Some(ef) = edit_form {
        let es = slugify(&ef.name.node);
        url_vals.push(format!("        val editUrl = \"/{es}?id=\" ++ pkVal"));
        link_parts.push("\"<a href=\\\"\" ++ editUrl ++ \"\\\">Edit</a>\"".to_string());
    }

    for (idx, cv) in child_views.iter().enumerate() {
        let cv_slug      = slugify(&cv.name.node);
        let cv_label     = to_title(&cv.name.node);
        let filter_param = cv.filter_by.as_deref().map(camel_to_snake).unwrap_or_default();
        let url_var      = format!("childUrl{idx}");
        url_vals.push(format!("        val {url_var} = \"/{cv_slug}?{filter_param}=\" ++ pkVal"));
        link_parts.push(format!("\"<a href=\\\"\" ++ {url_var} ++ \"\\\">{cv_label}</a>\""));
    }

    for uv in &url_vals {
        writeln!(out, "{uv}").unwrap();
    }

    let links_expr = if link_parts.is_empty() {
        "\"\"".to_string()
    } else {
        link_parts.join(" ++ \" &nbsp; \" ++ ")
    };

    writeln!(out, "        val links = {links_expr}").unwrap();
    writeln!(out, "        {fn_name}(cols, rows, i + 1, acc ++ \"<div class=\\\"row-actions\\\">\" ++ links ++ \"</div>\")").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

// ── Form GET handler ──────────────────────────────────────────────────────────

fn emit_form_get_handler(f: &FormDecl, module: &Module, out: &mut String) {
    let name    = &f.name.node;
    let fn_name = format!("handle{}Get", name);
    let slug    = slugify(name);
    let title   = to_title(name);
    let is_edit = name.to_lowercase().starts_with("edit");

    let target = f.target.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or(name.as_str());
    let list_slug  = slugify(&format!("{}List", target));
    let table      = camel_to_snake(target);

    writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();

    if is_edit {
        if let Some(ref pk_field) = f.pk {
            let pk_col = camel_to_snake(pk_field);
            writeln!(out, "    val _qry  = HttpRequest.query(req)").unwrap();
            writeln!(out, "    val _id   = formField(_qry, \"id\")").unwrap();
            writeln!(out, "    val conn  = dbConnect(dbUrl())").unwrap();
            writeln!(out, "    val _p0   = List.empty()").unwrap();
            writeln!(out, "    val _p1   = List.push(_p0, _id)").unwrap();
            writeln!(out, "    val _rows = dbQuery(conn, \"SELECT * FROM {table} WHERE {pk_col} = $1 LIMIT 1\", _p1)").unwrap();
            writeln!(out, "    val _cols = dbColumns(conn, \"SELECT * FROM {table} LIMIT 1\")").unwrap();
            writeln!(out, "    dbClose(conn)").unwrap();

            // Emit each field as its own val to avoid deep ++ chains
            for (i, field) in f.fields.iter().enumerate() {
                let fcol    = camel_to_snake(&field.name.node);
                let prefill = format!("colValue(_rows, _cols, \"{fcol}\")");
                let max_len = bounded_text_max_len(module, target, &field.name.node);
                let expr    = field_html_expr(field, Some(&prefill), max_len);
                writeln!(out, "    val _ef{i} = {expr}").unwrap();
            }

            // Build body by joining field vars in small chains
            let n = f.fields.len();
            writeln!(out, "    val _hdr = \"<h1>{title}</h1>\" ++ \"<nav><a href=\\\"/{list_slug}\\\">← Back to list</a></nav>\" ++ \"<form method=\\\"POST\\\" action=\\\"/{slug}?id=\\\" \" ++ _id ++ \"\\\">\"").unwrap();
            emit_field_concat_tree(out, n, "    ");
            writeln!(out, "    val _footer = \"<button type=\\\"submit\\\">Save changes</button>\" ++ \"</form>\"").unwrap();
            writeln!(out, "    val body = _hdr ++ _fields ++ _footer").unwrap();
            writeln!(out, "    Http.ok(htmlPage(\"{title}\", body), \"text/html\")").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
            return;
        }
    }

    // Create form (no pre-fill) — emit each field as its own val
    for (i, field) in f.fields.iter().enumerate() {
        let max_len = bounded_text_max_len(module, target, &field.name.node);
        let expr = field_html_expr(field, None, max_len);
        writeln!(out, "    val _ef{i} = {expr}").unwrap();
    }

    let n = f.fields.len();
    writeln!(out, "    val _hdr = \"<h1>{title}</h1>\" ++ \"<nav><a href=\\\"/{list_slug}\\\">← Back to list</a></nav>\" ++ \"<form method=\\\"POST\\\" action=\\\"/{slug}\\\">\"").unwrap();
    emit_field_concat_tree(out, n, "    ");
    writeln!(out, "    val _footer = \"<button type=\\\"submit\\\">Submit</button>\" ++ \"</form>\"").unwrap();
    writeln!(out, "    val body = _hdr ++ _fields ++ _footer").unwrap();
    writeln!(out, "    Http.ok(htmlPage(\"{title}\", body), \"text/html\")").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

// ── colValue helper (emitted once) ────────────────────────────────────────────

fn emit_col_value_helper(out: &mut String) {
    writeln!(out, "// ── Pre-fill helper: get a column value from the first row ──────────────────").unwrap();
    writeln!(out, "fn colValue(rows: List<List<Text?>>, cols: List<Text>, col: Text): Text =").unwrap();
    writeln!(out, "    if List.len(rows) == 0 then \"\"").unwrap();
    writeln!(out, "    else {{").unwrap();
    writeln!(out, "        val row = List.getOrPanic(rows, 0)").unwrap();
    writeln!(out, "        val idx = pkColIndex(cols, col, 0)").unwrap();
    writeln!(out, "        if idx < 0 then \"\" else (List.getOrPanic(row, idx) ?? \"\")").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

/// Emit `val _fields = _ef0 ++ _ef1 ++ ...` using a flat chain of at most
/// 4 terms per `val` so the parser never recurses more than a few levels deep.
fn emit_field_concat_tree(out: &mut String, n: usize, indent: &str) {
    if n == 0 {
        writeln!(out, "{indent}val _fields = \"\"").unwrap();
        return;
    }
    // Group _ef vars into chunks of 4, each assigned to an intermediate val
    let vars: Vec<String> = (0..n).map(|i| format!("_ef{i}")).collect();
    let mut level_vars = vars;
    let mut pass = 0usize;
    while level_vars.len() > 1 {
        let mut next: Vec<String> = vec![];
        for chunk in level_vars.chunks(4) {
            let joined = chunk.join(" ++ ");
            let var = format!("_fc{pass}_{}", next.len());
            writeln!(out, "{indent}val {var} = {joined}").unwrap();
            next.push(var);
        }
        level_vars = next;
        pass += 1;
    }
    // Rename the single survivor to _fields
    let survivor = &level_vars[0];
    writeln!(out, "{indent}val _fields = {survivor}").unwrap();
}

// ── Form POST handler ─────────────────────────────────────────────────────────

fn emit_form_post_handler(f: &FormDecl, has_live: bool, out: &mut String) {
    let name    = &f.name.node;
    let fn_name = format!("handle{}Post", name);
    let target  = f.target.segments.last()
        .map(|s| s.node.as_str())
        .unwrap_or(name.as_str());
    let table     = camel_to_snake(target);
    let list_slug = slugify(&format!("{}List", target));
    let is_edit   = name.to_lowercase().starts_with("edit");

    let cols: Vec<String> = f.fields.iter()
        .map(|field| camel_to_snake(&field.name.node))
        .collect();

    writeln!(out, "fn {fn_name}(req: HttpRequest): HttpResponse = {{").unwrap();
    writeln!(out, "    val body = HttpRequest.body(req)").unwrap();
    writeln!(out, "    val conn = dbConnect(dbUrl())").unwrap();

    for field in &f.fields {
        let fname = &field.name.node;
        writeln!(out, "    val {fname} = formField(body, \"{fname}\")").unwrap();
    }

    writeln!(out, "    val _p0 = List.empty()").unwrap();
    for (i, field) in f.fields.iter().enumerate() {
        let fname = &field.name.node;
        writeln!(out, "    val _p{} = List.push(_p{}, {})", i + 1, i, fname).unwrap();
    }

    if is_edit {
        if let Some(ref pk_field) = f.pk {
            let pk_col = camel_to_snake(pk_field);
            let n      = f.fields.len();
            // Extract id from query string
            writeln!(out, "    val _qry = HttpRequest.query(req)").unwrap();
            writeln!(out, "    val _id  = formField(_qry, \"id\")").unwrap();
            writeln!(out, "    val _p{} = List.push(_p{}, _id)", n + 1, n).unwrap();
            let last_p    = format!("_p{}", n + 1);
            let set_clause = cols.iter().enumerate()
                .map(|(i, c)| format!("{} = ${}", c, i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("UPDATE {table} SET {set_clause} WHERE {pk_col} = ${}", n + 1);
            writeln!(out, "    val _rows = dbExec(conn, \"{sql}\", {last_p})").unwrap();
            writeln!(out, "    dbClose(conn)").unwrap();
            if has_live {
                writeln!(out, "    if _rows > 0 then Http.liveNotify() else ()").unwrap();
            }
            writeln!(out, "    val msg = if _rows > 0 then \"Record updated.\" else \"Update failed — record not found.\"").unwrap();
            writeln!(out, "    val pg  = \"<h2>\" ++ msg ++ \"</h2><p><a href=\\\"/{list_slug}\\\">← Back to list</a></p>\"").unwrap();
            writeln!(out, "    Http.ok(htmlPage(\"Done\", pg), \"text/html\")").unwrap();
            writeln!(out, "}}").unwrap();
            writeln!(out).unwrap();
            return;
        }
    }

    // INSERT path
    let col_list  = cols.join(", ");
    let ph_list   = (1..=cols.len()).map(|i| format!("${i}")).collect::<Vec<_>>().join(", ");
    let sql       = format!("INSERT INTO {table} ({col_list}) VALUES ({ph_list})");
    let last_p    = format!("_p{}", f.fields.len());

    writeln!(out, "    val _rows = dbExec(conn, \"{sql}\", {last_p})").unwrap();
    writeln!(out, "    dbClose(conn)").unwrap();
    if has_live {
        writeln!(out, "    if _rows > 0 then Http.liveNotify() else ()").unwrap();
    }
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

/// The HTML `<input type="...">` value, or the sentinel "select"/"textarea"
/// for a widget that isn't a plain `<input>` at all (BACKLOG item 166) — an
/// explicit `type:` widget descriptor (from either the nested `field NAME {
/// type: ... }` block or the flat `fieldName: TypeExpr` shorthand, which
/// share the same `field_type` AST slot) takes priority; falling through to
/// the pre-existing field-name heuristic when absent, unchanged.
fn infer_input_type(name: &str, field_type: &Option<certo_ast::span::S<certo_ast::expr::Expr>>) -> &'static str {
    if let Some(ft) = field_type {
        use certo_ast::expr::Expr;
        let head = match &ft.node {
            Expr::Path { path, .. } => path.segments.last().map(|s| s.node.as_str()),
            Expr::App { func, .. } => match &func.node {
                Expr::Path { path, .. } => path.segments.last().map(|s| s.node.as_str()),
                _ => None,
            },
            _ => None,
        };
        if let Some(h) = head {
            match h.to_lowercase().as_str() {
                "select" | "dropdown"                  => return "select",
                "richtext"                              => return "textarea",
                "email"                                 => return "email",
                "password"                               => return "password",
                "phone" | "tel"                         => return "tel",
                "date"                                   => return "date",
                "url" | "website"                        => return "url",
                "time"                                   => return "time",
                "number"                                 => return "number",
                "currencyinput" | "currency" | "money"  => return "number",
                "color" | "colour"                       => return "color",
                "checkbox"                               => return "checkbox",
                "range"                                  => return "range",
                _ => {}
            }
        }
    }

    let lower = name.to_lowercase();
    if lower.contains("email")                          { return "email"; }
    if lower.contains("password")                       { return "password"; }
    if lower.contains("phone") || lower.contains("tel") { return "tel"; }
    if lower.contains("date")                           { return "date"; }
    if lower.contains("url") || lower.contains("website") { return "url"; }
    "text"
}

/// The `BoundedText(n)` bound declared for `field_name` on `target_type` in
/// `module`, if any — unwraps a `?`-suffixed optional field too, since
/// `BoundedText(n)?` still bounds every non-null value the same way.
/// BACKLOG item 272 — the real, mechanical half of spec §10.3's own
/// "auto-derived validation" claim: a client-side `maxlength` hint only,
/// matching `BoundedText`'s own documented design (`TypeExpr::
/// BoundedTextParam`'s doc comment, `crates/ast/src/types.rs`) as a
/// compile-time refinement with "no runtime length check... performed
/// anywhere" — adding real server-side rejection here would make this
/// generated handler the first runtime enforcement point for `BoundedText`
/// anywhere in the compiler, confirmed with the user to be a bigger,
/// separate step not taken here. The spec's other claim in the same
/// example ("Min(0) auto-derived from Money type constraints") is not
/// attempted at all — `Money` (spec §8.4: `{amount: Decimal(19,4),
/// currency: Currency}`) has no non-negative bound anywhere in the type
/// system to derive from, and legitimately goes negative in the domain
/// (refunds, discounts) — confirmed with the user this isn't a real
/// derivation to build.
fn bounded_text_max_len(module: &Module, target_type: &str, field_name: &str) -> Option<u32> {
    let fields = find_type_fields(module, target_type)?;
    let field = fields.iter().find(|f| f.name.node == field_name)?;
    bounded_text_max_len_of(&field.ty.node)
}

fn bounded_text_max_len_of(te: &TypeExpr) -> Option<u32> {
    match te {
        TypeExpr::BoundedTextParam { max_len, .. } => Some(*max_len),
        TypeExpr::Option { inner, .. } => bounded_text_max_len_of(&inner.node),
        _ => None,
    }
}

/// One field's `<div class="field">...</div>` as a *Certo source
/// expression* (this whole function generates text that gets compiled as
/// part of the handler, not real HTML directly) — BACKLOG item 166.
/// `prefill`, when given, is a raw Certo expression snippet (e.g.
/// `colValue(_rows, _cols, "col")`) spliced in via `++` for the edit form's
/// pre-filled value; `None` for the create form, which has nothing to fill.
/// `max_len`, when given (BACKLOG item 272), renders a `maxlength="n"`
/// attribute derived from the target type's own `BoundedText(n)` field.
fn field_html_expr(field: &FormField, prefill: Option<&str>, max_len: Option<u32>) -> String {
    let fname = &field.name.node;
    let label = field.label.as_deref().unwrap_or(fname.as_str());
    let itype = infer_input_type(fname, &field.field_type);
    let head  = format!("<div class=\\\"field\\\"><label for=\\\"{fname}\\\">{label}</label>");
    let maxlen_attr = max_len.map(|n| format!(" maxlength=\\\"{n}\\\"")).unwrap_or_default();

    if itype == "select" {
        // BACKLOG item 166 — `options:` is parsed (`FormField.options`) but
        // not yet rendered: a real live-query-backed `<select>` needs the
        // generated handler to run a DB query before rendering, its own
        // separate follow-on. Pre-fill is meaningless with no `<option>`s
        // to mark `selected`, so `prefill` is intentionally unused here.
        return format!(
            "\"{head}<!-- TODO: populate <select> options for '{fname}' -->\" ++ \"<select id=\\\"{fname}\\\" name=\\\"{fname}\\\"></select></div>\""
        );
    }

    if itype == "textarea" || field.rows.is_some_and(|r| r > 1) {
        let rows = field.rows.unwrap_or(3);
        return match prefill {
            Some(pf) => format!(
                "\"{head}<textarea id=\\\"{fname}\\\" name=\\\"{fname}\\\" rows=\\\"{rows}\\\"{maxlen_attr}>\" ++ {pf} ++ \"</textarea></div>\""
            ),
            None => format!(
                "\"{head}<textarea id=\\\"{fname}\\\" name=\\\"{fname}\\\" rows=\\\"{rows}\\\"{maxlen_attr}></textarea></div>\""
            ),
        };
    }

    match prefill {
        Some(pf) => format!(
            "\"{head}<input id=\\\"{fname}\\\" name=\\\"{fname}\\\" type=\\\"{itype}\\\"{maxlen_attr} value=\\\"\" ++ {pf} ++ \"\\\" required></div>\""
        ),
        None => format!(
            "\"{head}<input id=\\\"{fname}\\\" name=\\\"{fname}\\\" type=\\\"{itype}\\\"{maxlen_attr} required></div>\""
        ),
    }
}

// ------------------------------------------------------------------ //
// `@ui.generate` lowering (BACKLOG item 87)
// ------------------------------------------------------------------ //

fn find_type_fields<'a>(module: &'a Module, type_name: &str) -> Option<&'a [RecordFieldDef]> {
    module.decls.iter().find_map(|d| match &d.node {
        Decl::Type(t) if t.name.node == type_name => match &t.body {
            TypeBody::Record(r) => Some(r.fields.as_slice()),
            _ => None,
        },
        _ => None,
    })
}

/// Lower one `@ui.generate(Type) { title, list: { columns } }` into real
/// `view`/`form` declarations, by generating Certo *source text* in the
/// same shape a hand-written CRUD UI already uses (see e.g.
/// `examples/fireworks_ui.cto`'s `view CustomerList { ... For(customers,
/// customer) }` + `form CreateCustomer -> Customer { ... }` pair) and
/// re-parsing it — reusing the real parser instead of hand-building AST
/// nodes, and guaranteeing the output is exactly as valid as anything a
/// user could type themselves.
///
/// Scope notes (BACKLOG item 87 — each of these is a real, currently-
/// missing capability, not just an unwired flag):
/// - The list view always `SELECT *`s and displays every DB column
///   (`emit_view_handler`'s existing behavior, unchanged) — `list.columns`
///   is honored for the *create/edit forms'* field set, not to restrict
///   the rendered table, since no column-restriction mechanism exists in
///   the list-view codegen at all.
/// - An edit form is only generated when the type has a field literally
///   named `id` (case-insensitive) — used as the primary key. There's no
///   way to know a type's real primary key otherwise (Certo's `type X =
///   { ... }` doesn't mark one), and guessing wrong would generate a form
///   that edits/looks up the wrong row, a real correctness risk, not just
///   a missing feature — so it's skipped rather than guessed.
fn lower_ui_generate(g: &UiGenerateDecl, module: &Module) -> Result<Vec<S<Decl>>, UiError> {
    let type_name = &g.type_name.node;
    let fields = find_type_fields(module, type_name).ok_or_else(|| {
        UiError::ParseError(format!(
            "@ui.generate({type_name}): no `type {type_name} = {{ ... }}` record declaration found in this module"
        ))
    })?;

    let all_field_names: Vec<String> = fields.iter().map(|f| f.name.node.clone()).collect();
    let form_fields: Vec<String> = if g.columns.is_empty() { all_field_names.clone() } else { g.columns.clone() };
    let id_field = all_field_names.iter().find(|f| f.eq_ignore_ascii_case("id")).cloned();

    let title = g.title.clone().unwrap_or_else(|| format!("{type_name}s"));
    let lower = type_name.to_lowercase();

    let mut src = String::new();
    src.push_str("module _UiGenerate\n");
    src.push_str(&format!(
        "view {type_name}List {{\n    layout = VStack(children: [\n        Heading(\"{title}\"),\n        HStack(children: [\n            Button(\"New {type_name}\", \"/create-{lower}\"),\n        ]),\n        For(items, {type_name}),\n    ])\n}}\n"
    ));
    src.push_str(&format!("form Create{type_name} -> {type_name} {{\n"));
    for f in &form_fields {
        src.push_str(&format!("    {f}: Text,\n"));
    }
    src.push_str("}\n");
    if let Some(id) = &id_field {
        src.push_str(&format!("form Edit{type_name} -> {type_name} {{\n    pk: {id},\n"));
        for f in &form_fields {
            if f != id {
                src.push_str(&format!("    {f}: Text,\n"));
            }
        }
        src.push_str("}\n");
    }

    let parsed = certo_parser::parse(&src).map_err(|errs| {
        UiError::ParseError(format!(
            "@ui.generate({type_name}) produced invalid internal source (this is a codegen bug, not a problem with your annotation): {errs:?}"
        ))
    })?;
    Ok(parsed.decls)
}

// This file had zero test coverage before item 88 (BACKLOG's own note on
// that item flagged it explicitly) — starting real coverage here, not just
// for the `live val` comment this item's own change touches.
#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;

    fn server_source(src: &str) -> String {
        let m = parse(src).expect("parse");
        emit_server(&m).expect("emit_server")
    }

    #[test]
    fn live_val_emits_wired_comment_naming_the_binding() {
        // BACKLOG item 88 (stage 3) — a `live val` view is now genuinely
        // wired to the SSE push channel, not just "declared but not yet
        // wired" — the comment must reflect that.
        let out = server_source(
            "module M\nview Home {\n live val count = 0\n layout = Text(\"Hi\")\n}"
        );
        assert!(
            out.contains("// live bindings wired to SSE auto-refresh via /__certo_live: count"),
            "missing live-binding comment\n{}", out
        );
    }

    #[test]
    fn multiple_live_vals_all_named_in_comment() {
        let out = server_source(
            "module M\nview Home {\n live val a = 1\n live val b = \"x\"\n layout = Text(\"Hi\")\n}"
        );
        assert!(out.contains("a, b"), "missing both binding names\n{}", out);
    }

    #[test]
    fn live_val_view_embeds_an_sse_event_source_script() {
        let out = server_source(
            "module M\nview Home {\n live val count = 0\n layout = Text(\"Hi\")\n}"
        );
        assert!(out.contains("EventSource('/__certo_live')"), "missing the SSE client script\n{}", out);
        assert!(out.contains("location.reload()"), "missing the refresh-on-signal call\n{}", out);
    }

    #[test]
    fn view_without_live_val_has_no_sse_script() {
        let out = server_source("module M\nview Home {\n layout = Text(\"Hi\")\n}");
        assert!(!out.contains("EventSource"), "unexpected SSE client script on a non-live view\n{}", out);
    }

    #[test]
    fn form_post_handler_notifies_after_a_successful_write_when_a_live_view_exists() {
        let out = server_source(
            "module M\nview Home {\n live val count = 0\n layout = Text(\"Hi\")\n}\nform CreateWidget -> Widget {\n name: Text\n}"
        );
        assert!(out.contains("Http.liveNotify()"), "expected the create-form handler to notify on write\n{}", out);
    }

    #[test]
    fn form_post_handler_does_not_notify_with_no_live_views_in_the_module() {
        let out = server_source("module M\nform CreateWidget -> Widget {\n name: Text\n}");
        assert!(!out.contains("Http.liveNotify"), "should not emit a notify call with no live val anywhere\n{}", out);
    }

    #[test]
    fn bounded_text_field_gets_maxlength_attribute() {
        // BACKLOG item 272 — the real, mechanical half of spec §10.3's own
        // "auto-derived validation" claim: a client-side `maxlength` hint
        // derived from the target type's own `BoundedText(n)` field.
        let out = server_source(
            "module M\ntype Product = { name: BoundedText(5) }\nform CreateProduct -> Product {\n field name {\n label: \"Name\"\n }\n}"
        );
        assert!(out.contains("maxlength=\\\"5\\\""), "expected a maxlength=\\\"5\\\" attribute\n{}", out);
    }

    #[test]
    fn optional_bounded_text_field_still_gets_maxlength() {
        let out = server_source(
            "module M\ntype Product = { description: BoundedText(2000)? }\nform CreateProduct -> Product {\n field description {\n label: \"Description\"\n }\n}"
        );
        assert!(out.contains("maxlength=\\\"2000\\\""), "expected an optional BoundedText field to still carry maxlength\n{}", out);
    }

    #[test]
    fn non_bounded_field_has_no_maxlength_attribute() {
        let out = server_source(
            "module M\ntype Product = { price: Decimal(10,2) }\nform CreateProduct -> Product {\n field price {\n label: \"Price\"\n }\n}"
        );
        assert!(!out.contains("maxlength"), "a plain Decimal field must not get a maxlength attribute\n{}", out);
    }

    #[test]
    fn bounded_text_maxlength_appears_on_textarea_too() {
        let out = server_source(
            "module M\ntype Product = { description: BoundedText(500) }\nform CreateProduct -> Product {\n field description {\n label: \"Description\"\n type: RichText\n rows: 6\n }\n}"
        );
        assert!(out.contains("<textarea") && out.contains("maxlength=\\\"500\\\""),
            "expected the RichText textarea to carry maxlength too\n{}", out);
    }

    #[test]
    fn form_without_a_matching_type_declaration_has_no_maxlength() {
        // Regression guard: a form field with no corresponding `type`
        // declaration in the module (already-existing tests above use this
        // exact shape) must not error or spuriously add a maxlength.
        let out = server_source("module M\nform CreateWidget -> Widget {\n name: Text\n}");
        assert!(!out.contains("maxlength"), "no type declaration exists for Widget — nothing to derive from\n{}", out);
    }

    #[test]
    fn unrecognized_layout_primitive_leaves_a_visible_diagnostic_comment() {
        // BACKLOG item 215 — this handler previously stripped every line
        // starting with `<!--` before splicing `layout_to_html`'s output
        // in, silently discarding the diagnostic comment `layout_to_html`
        // itself emits for an unrecognized primitive (`layout.rs`'s own
        // "unknown UI primitive: {name}" case) — the gap vanished with zero
        // trace in the page this handler actually serves. The legacy
        // `--html` codegen mode already kept it; this closes the gap in
        // the default Htmx path too.
        let out = server_source(
            "module M\nview Home {\n layout = Mystery(\"x\")\n}"
        );
        assert!(
            out.contains("unknown UI primitive: Mystery"),
            "diagnostic comment for an unrecognized layout primitive vanished\n{}", out
        );
    }

    #[test]
    fn view_without_live_val_has_no_live_comment() {
        // No regression: a view that never declares `live val` must not
        // grow a stray comment about bindings that don't exist.
        let out = server_source("module M\nview Home {\n layout = Text(\"Hi\")\n}");
        assert!(!out.contains("live bindings"), "unexpected live-binding comment\n{}", out);
    }

    // ------------------------------------------------------------------ //
    // Nested `field NAME { type: ... }` widget kinds (BACKLOG item 166)
    // ------------------------------------------------------------------ //

    #[test]
    fn nested_field_type_select_renders_select_in_create_form() {
        let out = server_source(
            "module M\nform CreateWidget -> Widget {\n field categoryId { type: Select }\n}"
        );
        assert!(out.contains("<select id=\\\"categoryId\\\" name=\\\"categoryId\\\">"), "expected <select>, got:\n{out}");
        assert!(out.contains("TODO: populate <select> options"), "expected the options TODO comment, got:\n{out}");
        assert!(!out.contains("<input"), "should not also render an <input> for a Select field, got:\n{out}");
    }

    #[test]
    fn nested_field_type_richtext_renders_textarea_with_default_rows() {
        let out = server_source(
            "module M\nform CreateWidget -> Widget {\n field description { type: RichText }\n}"
        );
        assert!(out.contains("<textarea"), "expected <textarea>, got:\n{out}");
        assert!(out.contains("rows=\\\"3\\\""), "expected the default rows=3 (no `rows:` key given), got:\n{out}");
    }

    #[test]
    fn nested_field_type_currency_input_renders_numeric_input() {
        let out = server_source(
            "module M\nform CreateWidget -> Widget {\n field price { type: CurrencyInput(USD) }\n}"
        );
        assert!(out.contains("type=\\\"number\\\""), "expected a numeric <input>, got:\n{out}");
    }

    #[test]
    fn edit_form_textarea_field_prefills_via_concatenation_not_a_value_attribute() {
        // A textarea's pre-filled content is text *between* its open/close
        // tags, not a `value="..."` attribute the way <input> pre-fills.
        let out = server_source(
            "module M\nform EditWidget -> Widget {\n pk: id\n field description { type: RichText }\n}"
        );
        assert!(out.contains("<textarea"), "expected <textarea> in edit form, got:\n{out}");
        assert!(out.contains("++ colValue("), "expected the pre-fill value spliced in via `++`, got:\n{out}");
    }

    #[test]
    fn edit_form_select_field_has_no_prefill_expression() {
        // No `<option>`s exist yet (options: is parsed but not rendered,
        // BACKLOG item 166's own scope decision) so there's nothing to mark
        // `selected` — a Select field's edit-form line must not attempt a
        // `colValue` pre-fill splice at all.
        let out = server_source(
            "module M\nform EditWidget -> Widget {\n pk: id\n field categoryId { type: Select }\n}"
        );
        let select_line = out.lines().find(|l| l.contains("<select")).expect("expected a <select> line");
        assert!(!select_line.contains("colValue"), "a Select field should not attempt to pre-fill, got:\n{select_line}");
    }

    #[test]
    fn flat_and_nested_fields_coexist_in_generated_form() {
        let out = server_source(
            "module M\nform CreateWidget -> Widget {\n name: Text\n field description { type: RichText }\n}"
        );
        assert!(out.contains("\"name\""), "expected the flat `name` field, got:\n{out}");
        assert!(out.contains("<textarea"), "expected the nested `description` field's textarea, got:\n{out}");
    }

    // ------------------------------------------------------------------ //
    // `@ui.generate` lowering (BACKLOG item 87)
    // ------------------------------------------------------------------ //

    const PRODUCT_SRC: &str = "\
type Product = { id: Text, name: Text, sku: Text }
@ui.generate(Product) {
    title: \"Products\"
    list: { columns: [name, sku] }
}
";

    #[test]
    fn ui_generate_creates_list_route_and_create_form() {
        let out = server_source(&format!("module M\n{PRODUCT_SRC}"));
        assert!(out.contains("fn handleProductList"), "missing list handler\n{out}");
        assert!(out.contains("/product-list"), "missing list route\n{out}");
        assert!(out.contains("fn handleCreateProductGet"), "missing create-get handler\n{out}");
        assert!(out.contains("fn handleCreateProductPost"), "missing create-post handler\n{out}");
        // list.columns determines the create form's fields, not the list
        // table's displayed columns (no such restriction mechanism exists
        // in emit_view_handler — see lower_ui_generate's own doc comment).
        assert!(out.contains("\"name\""), "create form missing `name` field\n{out}");
        assert!(out.contains("\"sku\""), "create form missing `sku` field\n{out}");
    }

    #[test]
    fn ui_generate_with_id_field_also_creates_edit_form() {
        let out = server_source(&format!("module M\n{PRODUCT_SRC}"));
        assert!(out.contains("fn handleEditProductGet"), "missing edit-get handler\n{out}");
        assert!(out.contains("fn handleEditProductPost"), "missing edit-post handler\n{out}");
        assert!(out.contains("/edit-product"), "missing edit route\n{out}");
    }

    #[test]
    fn ui_generate_without_id_field_skips_edit_form() {
        // No reliable way to know a type's real primary key without an
        // `id` field — generating an edit form anyway would risk editing
        // the wrong row, so it's skipped rather than guessed.
        let out = server_source(
            "module M\ntype Widget = { name: Text, color: Text }\n@ui.generate(Widget) {}"
        );
        assert!(out.contains("fn handleWidgetList"), "missing list handler\n{out}");
        assert!(out.contains("fn handleCreateWidgetGet"), "missing create handler\n{out}");
        assert!(!out.contains("handleEditWidget"), "must not fabricate an edit form with no known pk\n{out}");
    }

    #[test]
    fn ui_generate_defaults_title_and_uses_all_fields_when_unspecified() {
        let out = server_source(
            "module M\ntype Widget = { id: Text, name: Text, color: Text }\n@ui.generate(Widget) {}"
        );
        assert!(out.contains("Widget List") || out.contains("Widgets"), "expected a default title\n{out}");
        assert!(out.contains("\"name\"") && out.contains("\"color\""), "expected all fields used as form fields by default\n{out}");
    }

    #[test]
    fn ui_generate_referencing_unknown_type_is_a_clear_error() {
        let m = parse("module M\n@ui.generate(Nonexistent) {}").expect("parse");
        let err = emit_server(&m).expect_err("expected an error for an unknown type");
        assert!(matches!(err, UiError::ParseError(ref s) if s.contains("Nonexistent")), "got: {err:?}");
    }

    #[test]
    fn ui_generate_output_never_calls_list_empty_as_a_bare_value() {
        // Regression test for a real bug found via end-to-end testing
        // (BACKLOG item 87's own verification): `List.empty` is a zero-arg
        // *function* (`Ty::Fn { params: vec![], .. }`, `crates/stdlib/src/
        // seed.rs`), not a value — used bare (no `()`) at 4 call sites in
        // this file, it type-checked as "expected a function, found
        // List<T>" and failed to compile. Pre-existing, not specific to
        // `@ui.generate` (any hand-written view+form pair with an
        // unfiltered list and a form hits the same code paths), but only
        // surfaced once something actually compiled the generated output
        // instead of just eyeballing it.
        let out = server_source(&format!("module M\n{PRODUCT_SRC}"));
        for line in out.lines() {
            if let Some(idx) = line.find("List.empty") {
                let after = &line[idx + "List.empty".len()..];
                assert!(after.starts_with('('), "found bare `List.empty` (missing call parens): {line}");
            }
        }
    }
}
