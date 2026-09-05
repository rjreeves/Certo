//! Compile a Certo layout expression to an HTML fragment.
//!
//! Recognised UI primitives:
//!
//! | Certo call             | HTML output                            |
//! |------------------------|----------------------------------------|
//! | `VStack(children: […])`| `<div class="vstack">…</div>`          |
//! | `HStack(children: […])`| `<div class="hstack">…</div>`          |
//! | `Text(value)`          | `<p>…</p>`                             |
//! | `Heading(value)`       | `<h2>…</h2>`                           |
//! | `Button(label, href?)` | `<button hx-get="…">…</button>`        |
//! | `Link(label, href)`    | `<a href="…">…</a>`                    |
//! | `Image(src, alt?)`     | `<img src="…" alt="…">`               |
//! | `Input(name, label?)`  | `<label>…<input name="…"></label>`     |
//! | `For(items, body)`     | comment noting dynamic list            |
//! | `If(cond, body)`       | comment noting conditional             |
//!
//! BACKLOG item 216 — a deliberately bounded, **static-only** dashboard
//! vocabulary (spec §10.2's own dashboard primitives, minus the two pieces
//! the item's own investigation found substantially bigger and explicitly
//! declined for now: the `Foo(...) { block }` trailing-block call syntax,
//! and any live-query/`live val` data binding — every value rendered here
//! must be a compile-time literal, same restriction every existing
//! primitive above already has):
//!
//! | Certo call                                          | HTML output |
//! |------------------------------------------------------|-------------|
//! | `Column(children: […])`                               | `<div class="column">…</div>` — `VStack`'s own alias |
//! | `Row(children: […])`                                  | `<div class="row">…</div>` — `HStack`'s own alias |
//! | `MetricCard(label, value, format?, alert?)`           | `<div class="metric-card">…</div>` |
//! | `Table(columns: […], rows: […])`                      | a real `<table>` |
//!
//! Any unrecognised call emits an HTML comment so the file stays valid.

use std::fmt::Write as _;
use certo_ast::expr::{Expr, Arg, Lit};
use certo_ast::span::S;

/// Compile a layout `Expr` into an indented HTML string.
pub fn layout_to_html(expr: &Expr, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    match expr {
        Expr::App { func, args, .. } => {
            let name = call_name(func);
            match name.as_deref() {
                Some("VStack") => container_html("vstack", args, indent),
                Some("HStack") => container_html("hstack", args, indent),
                Some("Text")   => inline_html("p",  first_str_arg(args), &pad),
                Some("Heading")=> inline_html("h2", first_str_arg(args), &pad),
                Some("Link")   => link_html(args, &pad),
                Some("Button") => button_html(args, &pad),
                Some("Image")  => image_html(args, &pad),
                Some("Input")  => input_html(args, &pad),
                Some("For")    => format!("{pad}<!-- For loop: render list here -->\n"),
                Some("If")     => format!("{pad}<!-- If condition: render conditionally here -->\n"),
                // BACKLOG item 216 — static-only dashboard vocabulary.
                Some("Column")     => container_html("column", args, indent),
                Some("Row")        => container_html("row", args, indent),
                Some("MetricCard") => metric_card_html(args, &pad),
                Some("Table")      => table_html(args, indent),
                Some(other)    => format!("{pad}<!-- unknown UI primitive: {} -->\n", other),
                None           => format!("{pad}<!-- complex expression: cannot render -->\n"),
            }
        }
        Expr::Lit { value: Lit::String(s), .. } => {
            format!("{pad}<p>{}</p>\n", html_escape(s))
        }
        Expr::List { elements, .. } => {
            elements.iter().map(|it| layout_to_html(&it.node, indent)).collect()
        }
        _ => format!("{pad}<!-- expression: cannot render statically -->\n"),
    }
}

// ------------------------------------------------------------------ //
// Primitive renderers
// ------------------------------------------------------------------ //

fn container_html(class: &str, args: &[Arg], indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let children = children_from_args(args);
    let mut out = String::new();
    writeln!(out, "{pad}<div class=\"{class}\">").unwrap();
    for child in &children {
        out.push_str(&layout_to_html(&child.node, indent + 1));
    }
    writeln!(out, "{pad}</div>").unwrap();
    out
}

fn inline_html(tag: &str, text: Option<&str>, pad: &str) -> String {
    let content = text.map(html_escape).unwrap_or_default();
    format!("{pad}<{tag}>{content}</{tag}>\n")
}

fn link_html(args: &[Arg], pad: &str) -> String {
    let label = nth_str_arg(args, 0).unwrap_or("link");
    let href  = nth_str_arg(args, 1).unwrap_or("#");
    format!("{pad}<a href=\"{}\">{}</a>\n", html_escape(href), html_escape(label))
}

fn button_html(args: &[Arg], pad: &str) -> String {
    let label = nth_str_arg(args, 0).unwrap_or("Submit");
    let href  = nth_str_arg(args, 1);
    if let Some(h) = href {
        format!("{pad}<button hx-get=\"{}\" hx-swap=\"outerHTML\">{}</button>\n",
            html_escape(h), html_escape(label))
    } else {
        format!("{pad}<button type=\"button\">{}</button>\n", html_escape(label))
    }
}

fn image_html(args: &[Arg], pad: &str) -> String {
    let src = nth_str_arg(args, 0).unwrap_or("");
    let alt = nth_str_arg(args, 1).unwrap_or("");
    format!("{pad}<img src=\"{}\" alt=\"{}\">\n", html_escape(src), html_escape(alt))
}

fn input_html(args: &[Arg], pad: &str) -> String {
    let name  = nth_str_arg(args, 0).unwrap_or("field");
    let label = nth_str_arg(args, 1).unwrap_or(name);
    format!("{pad}<label>{}<input name=\"{}\" type=\"text\"></label>\n",
        html_escape(label), html_escape(name))
}

// BACKLOG item 216 — static-only dashboard vocabulary.

fn metric_card_html(args: &[Arg], pad: &str) -> String {
    let label  = nth_str_arg(args, 0).unwrap_or("Metric");
    let value  = nth_any_lit_arg(args, 1).unwrap_or_default();
    let format = named_arg_as_name(args, "format");
    let alert  = named_arg_as_bool(args, "alert");

    let mut card_class = "metric-card".to_string();
    if alert { card_class.push_str(" alert"); }
    let value_class = match &format {
        Some(f) => format!(" metric-value-{}", f.to_lowercase()),
        None    => String::new(),
    };
    format!(
        "{pad}<div class=\"{card_class}\">\n\
         {pad}  <div class=\"metric-label\">{}</div>\n\
         {pad}  <div class=\"metric-value{value_class}\">{}</div>\n\
         {pad}</div>\n",
        html_escape(label), html_escape(&value)
    )
}

fn table_html(args: &[Arg], indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let columns: Vec<String> = named_list_arg(args, "columns").iter()
        .filter_map(|e| lit_expr_to_string(&e.node))
        .collect();
    let rows: Vec<Vec<String>> = named_list_arg(args, "rows").iter()
        .map(|row| match &row.node {
            Expr::List { elements, .. } => elements.iter()
                .filter_map(|c| lit_expr_to_string(&c.node))
                .collect(),
            other => lit_expr_to_string(other).into_iter().collect(),
        })
        .collect();

    let mut out = String::new();
    writeln!(out, "{pad}<table>").unwrap();
    if !columns.is_empty() {
        writeln!(out, "{pad}  <thead>\n{pad}    <tr>").unwrap();
        for col in &columns {
            writeln!(out, "{pad}      <th>{}</th>", html_escape(col)).unwrap();
        }
        writeln!(out, "{pad}    </tr>\n{pad}  </thead>").unwrap();
    }
    writeln!(out, "{pad}  <tbody>").unwrap();
    for row in &rows {
        writeln!(out, "{pad}    <tr>").unwrap();
        for cell in row {
            writeln!(out, "{pad}      <td>{}</td>", html_escape(cell)).unwrap();
        }
        writeln!(out, "{pad}    </tr>").unwrap();
    }
    writeln!(out, "{pad}  </tbody>\n{pad}</table>").unwrap();
    out
}

// ------------------------------------------------------------------ //
// Argument helpers
// ------------------------------------------------------------------ //

fn call_name(func: &S<Expr>) -> Option<String> {
    if let Expr::Path { path, .. } = &func.node {
        path.segments.last().map(|s| s.node.clone())
    } else {
        None
    }
}

fn children_from_args(args: &[Arg]) -> Vec<S<Expr>> {
    // Named `children` arg with a list value
    for arg in args {
        if arg.label.as_ref().map(|l| l.node.as_str()) == Some("children") {
            if let Expr::List { elements, .. } = &arg.value.node {
                return elements.clone();
            }
        }
    }
    // Positional first arg that is a list
    if let Some(first) = args.first() {
        if let Expr::List { elements, .. } = &first.value.node {
            return elements.clone();
        }
    }
    vec![]
}

fn first_str_arg(args: &[Arg]) -> Option<&str> {
    nth_str_arg(args, 0)
}

fn nth_str_arg(args: &[Arg], n: usize) -> Option<&str> {
    // Positional args (no label)
    let positional: Vec<_> = args.iter().filter(|a| a.label.is_none()).collect();
    if let Some(arg) = positional.get(n) {
        if let Expr::Lit { value: Lit::String(s), .. } = &arg.value.node {
            return Some(s.as_str());
        }
    }
    // Named fallbacks for n==0
    if n == 0 {
        for key in &["value", "text", "label", "content"] {
            for arg in args {
                if arg.label.as_ref().map(|l| l.node.as_str()) == Some(key) {
                    if let Expr::Lit { value: Lit::String(s), .. } = &arg.value.node {
                        return Some(s.as_str());
                    }
                }
            }
        }
    }
    // Named fallbacks for n==1
    if n == 1 {
        for key in &["href", "src", "alt"] {
            for arg in args {
                if arg.label.as_ref().map(|l| l.node.as_str()) == Some(key) {
                    if let Expr::Lit { value: Lit::String(s), .. } = &arg.value.node {
                        return Some(s.as_str());
                    }
                }
            }
        }
    }
    None
}

/// Render any literal this static-only slice can meaningfully display as
/// plain text — `Text`/`Int`/`Float`/`Decimal`/`Bool` — or `None` for
/// anything else (an f-string, a live `val` reference, a whole expression).
/// BACKLOG item 216's own bounded scope: every value here must already be
/// known at compile time, same restriction `nth_str_arg` already enforces
/// for the primitives above, just generalized past bare strings so a
/// `MetricCard`'s numeric value or a `Table` cell isn't forced through a
/// string literal to render at all.
fn lit_expr_to_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit { value, .. } => match value {
            Lit::String(s)  => Some(s.clone()),
            Lit::Int(n)      => Some(n.to_string()),
            Lit::Float(f)    => Some(f.to_string()),
            Lit::Decimal(s)  => Some(s.clone()),
            Lit::Bool(b)     => Some(b.to_string()),
            Lit::FString(_) | Lit::Uuid(_) | Lit::Unit => None,
        },
        _ => None,
    }
}

/// Same positional/named-fallback lookup as `nth_str_arg`, but accepting any
/// literal `lit_expr_to_string` can render, not just `Lit::String`.
fn nth_any_lit_arg(args: &[Arg], n: usize) -> Option<String> {
    let positional: Vec<_> = args.iter().filter(|a| a.label.is_none()).collect();
    if let Some(arg) = positional.get(n) {
        if let Some(s) = lit_expr_to_string(&arg.value.node) {
            return Some(s);
        }
    }
    if n == 1 {
        for key in &["value"] {
            for arg in args {
                if arg.label.as_ref().map(|l| l.node.as_str()) == Some(*key) {
                    if let Some(s) = lit_expr_to_string(&arg.value.node) {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

/// A named arg's own "name" — either a bare identifier (`format: Currency`)
/// or a plain string (`format: "Currency"`); used for CSS-class-style
/// annotations rather than displayed text, so no HTML-escaping concern.
fn named_arg_as_name(args: &[Arg], key: &str) -> Option<String> {
    for arg in args {
        if arg.label.as_ref().map(|l| l.node.as_str()) == Some(key) {
            return match &arg.value.node {
                Expr::Path { path, .. } => path.segments.last().map(|s| s.node.clone()),
                Expr::Lit { value: Lit::String(s), .. } => Some(s.clone()),
                _ => None,
            };
        }
    }
    None
}

/// A named arg's literal `Bool` value (`alert: true`) — anything else
/// (a real comparison expression, a live value) isn't a compile-time literal
/// this static-only slice can evaluate, so it's treated as absent/`false`
/// rather than guessed at.
fn named_arg_as_bool(args: &[Arg], key: &str) -> bool {
    for arg in args {
        if arg.label.as_ref().map(|l| l.node.as_str()) == Some(key) {
            if let Expr::Lit { value: Lit::Bool(b), .. } = &arg.value.node {
                return *b;
            }
        }
    }
    false
}

/// The raw element expressions of a named list argument (`columns: [...]`),
/// or an empty list when the arg is missing or isn't a list literal.
fn named_list_arg(args: &[Arg], key: &str) -> Vec<S<Expr>> {
    for arg in args {
        if arg.label.as_ref().map(|l| l.node.as_str()) == Some(key) {
            if let Expr::List { elements, .. } = &arg.value.node {
                return elements.clone();
            }
        }
    }
    vec![]
}

// ------------------------------------------------------------------ //
// HTML escape
// ------------------------------------------------------------------ //

pub fn html_escape(s: &str) -> String {
    s.replace('&',  "&amp;")
     .replace('<',  "&lt;")
     .replace('>',  "&gt;")
     .replace('"',  "&quot;")
     .replace('\'', "&#39;")
}

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;
    use certo_ast::decl::Decl;

    fn layout_html(src: &str) -> String {
        use certo_ast::expr::{Expr, Stmt};
        let m = parse(src).expect("parse");
        for sd in &m.decls {
            if let Decl::View(v) = &sd.node {
                // layout is stored as Expr::Block { Stmt::Assign { target: "layout", value } }
                if let Expr::Block { stmts, .. } = &v.layout.node {
                    for stmt in stmts {
                        if let Stmt::Assign { target, value, .. } = stmt {
                            if target.node == "layout" {
                                return layout_to_html(&value.node, 0);
                            }
                        }
                    }
                }
                return layout_to_html(&v.layout.node, 0);
            }
        }
        panic!("no view found");
    }

    #[test]
    fn text_primitive() {
        let src = "module M\nview Foo { layout = Text(\"hello\") }\n";
        let html = layout_html(src);
        assert!(html.contains("<p>hello</p>"), "got: {}", html);
    }

    #[test]
    fn vstack_wraps_children() {
        let src = "module M\nview Foo { layout = VStack(children: [Text(\"a\"), Text(\"b\")]) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"vstack\""), "got: {}", html);
        assert!(html.contains("<p>a</p>"), "got: {}", html);
        assert!(html.contains("<p>b</p>"), "got: {}", html);
    }

    #[test]
    fn button_with_href_uses_hx_get() {
        let src = "module M\nview Foo { layout = Button(\"Click\", \"/action\") }\n";
        let html = layout_html(src);
        assert!(html.contains("hx-get=\"/action\""), "got: {}", html);
        assert!(html.contains("Click"), "got: {}", html);
    }

    #[test]
    fn button_without_href_is_plain() {
        let src = "module M\nview Foo { layout = Button(\"OK\") }\n";
        let html = layout_html(src);
        assert!(html.contains("type=\"button\""), "got: {}", html);
        assert!(!html.contains("hx-get"), "unexpected hx-get\n{}", html);
    }

    #[test]
    fn html_escaping() {
        let src = "module M\nview Foo { layout = Text(\"<b>&\") }\n";
        let html = layout_html(src);
        assert!(html.contains("&lt;b&gt;&amp;"), "got: {}", html);
    }

    #[test]
    fn unknown_primitive_becomes_comment() {
        let src = "module M\nview Foo { layout = Mystery(\"x\") }\n";
        let html = layout_html(src);
        assert!(html.contains("<!-- unknown UI primitive: Mystery -->"), "got: {}", html);
    }

    // ------------------------------------------------------------------ //
    // BACKLOG item 216 — static-only dashboard vocabulary
    // ------------------------------------------------------------------ //

    #[test]
    fn column_and_row_are_real_containers() {
        let src = "module M\nview Foo { layout = Column(children: [Row(children: [Text(\"a\")])]) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"column\""), "got: {}", html);
        assert!(html.contains("class=\"row\""), "got: {}", html);
        assert!(html.contains("<p>a</p>"), "got: {}", html);
    }

    #[test]
    fn metric_card_renders_label_and_literal_value() {
        let src = "module M\nview Foo { layout = MetricCard(\"Pending\", 42) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"metric-card\""), "got: {}", html);
        assert!(html.contains("metric-label\">Pending<"), "got: {}", html);
        assert!(html.contains("metric-value\">42<"), "got: {}", html);
    }

    #[test]
    fn metric_card_format_becomes_a_value_css_class() {
        let src = "module M\nview Foo { layout = MetricCard(\"Revenue\", 100, format: Currency) }\n";
        let html = layout_html(src);
        assert!(html.contains("metric-value-currency"), "got: {}", html);
    }

    #[test]
    fn metric_card_alert_true_adds_an_alert_class() {
        let src = "module M\nview Foo { layout = MetricCard(\"Low Stock\", 3, alert: true) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"metric-card alert\""), "got: {}", html);
    }

    #[test]
    fn metric_card_alert_non_literal_is_ignored_not_guessed() {
        // A real comparison expression (not a compile-time literal) is
        // outside this static-only slice's scope — must not panic, and
        // must not silently guess `true`.
        let src = "module M\nfn f(): Unit = {}\nview Foo { layout = MetricCard(\"Low Stock\", 3, alert: 3 > 0) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"metric-card\""), "got: {}", html);
        assert!(!html.contains("alert\""), "expected no alert class for a non-literal condition: {}", html);
    }

    #[test]
    fn table_renders_real_columns_and_rows() {
        let src = "module M\nview Foo { layout = Table(columns: [\"Name\", \"Amount\"], rows: [[\"Alice\", \"10.00\"], [\"Bob\", \"5.00\"]]) }\n";
        let html = layout_html(src);
        assert!(html.contains("<table>"), "got: {}", html);
        assert!(html.contains("<th>Name</th>"), "got: {}", html);
        assert!(html.contains("<th>Amount</th>"), "got: {}", html);
        assert!(html.contains("<td>Alice</td>"), "got: {}", html);
        assert!(html.contains("<td>10.00</td>"), "got: {}", html);
        assert!(html.contains("<td>Bob</td>"), "got: {}", html);
    }

    #[test]
    fn table_with_no_columns_still_renders_rows() {
        let src = "module M\nview Foo { layout = Table(rows: [[\"x\"]]) }\n";
        let html = layout_html(src);
        assert!(!html.contains("<thead>"), "expected no <thead> with no columns: {}", html);
        assert!(html.contains("<td>x</td>"), "got: {}", html);
    }

    #[test]
    fn dashboard_example_composes_all_four_new_primitives() {
        // The static, real-syntax slice of spec §10.2's own flagship
        // dashboard — no live queries, no trailing-block Table syntax.
        let src = "module M\nview Dashboard { layout = Column(children: [\n\
            Row(children: [MetricCard(\"Pending\", 12), MetricCard(\"Revenue\", 500, format: Currency)]),\n\
            Table(columns: [\"Order\", \"Status\"], rows: [[\"1001\", \"Shipped\"]])\n\
        ]) }\n";
        let html = layout_html(src);
        assert!(html.contains("class=\"column\""), "got: {}", html);
        assert!(html.contains("class=\"row\""), "got: {}", html);
        assert!(html.contains("metric-label\">Pending<"), "got: {}", html);
        assert!(html.contains("metric-value-currency"), "got: {}", html);
        assert!(html.contains("<th>Order</th>"), "got: {}", html);
        assert!(html.contains("<td>1001</td>"), "got: {}", html);
    }
}
