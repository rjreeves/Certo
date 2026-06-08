//! Generate a standalone HTML page from a `ViewDecl`.
//!
//! # Example
//!
//! ```certo
//! view Dashboard {
//!   layout = VStack(children: [
//!     Heading("Dashboard"),
//!     Text("Welcome back"),
//!     Button("Refresh", "/dashboard/refresh"),
//!   ])
//! }
//! ```
//!
//! Generates `dashboard.html`:
//!
//! ```html
//! <!DOCTYPE html>
//! <html lang="en">
//! <head>…<title>Dashboard</title>…</head>
//! <body>
//! <div id="dashboard-view" class="certo-view"
//!      hx-get="/dashboard" hx-trigger="load" hx-swap="innerHTML">
//!   <div class="vstack">
//!     <h2>Dashboard</h2>
//!     <p>Welcome back</p>
//!     <button hx-get="/dashboard/refresh" hx-swap="outerHTML">Refresh</button>
//!   </div>
//! </div>
//! </body>
//! </html>
//! ```

use std::fmt::Write as _;
use certo_ast::decl::ViewDecl;
use certo_ast::expr::{Expr, Stmt};
use crate::layout::layout_to_html;

/// Generate a full standalone HTML page for a single `ViewDecl`.
pub fn generate_view(v: &ViewDecl) -> String {
    let name      = &v.name.node;
    let slug      = slugify(name);
    let endpoint  = format!("/{}", slug);


    let layout_expr = extract_layout_expr(&v.layout.node)
        .unwrap_or(&v.layout.node);
    let layout_html = layout_to_html(layout_expr, 2);

    let live_comment = if v.live.is_empty() {
        String::new()
    } else {
        let bindings: Vec<_> = v.live.iter()
            .filter_map(|vd| {
                if let certo_ast::pattern::Pattern::Ident { name, .. } = &vd.pattern.node {
                    Some(name.node.clone())
                } else {
                    None
                }
            })
            .collect();
        format!("  <!-- live bindings: {} — wire to SSE or polling -->\n",
            bindings.join(", "))
    };

    let mut out = String::new();
    writeln!(out, "<!DOCTYPE html>").unwrap();
    writeln!(out, "<html lang=\"en\">").unwrap();
    writeln!(out, "<head>").unwrap();
    writeln!(out, "  <meta charset=\"UTF-8\">").unwrap();
    writeln!(out, "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">").unwrap();
    writeln!(out, "  <title>{}</title>", html_esc(name)).unwrap();
    writeln!(out, "  <script src=\"https://unpkg.com/htmx.org@2\" defer></script>").unwrap();
    writeln!(out, "  <style>").unwrap();
    writeln!(out, "    .vstack {{ display: flex; flex-direction: column; gap: 0.5rem; }}").unwrap();
    writeln!(out, "    .hstack {{ display: flex; flex-direction: row;    gap: 0.5rem; }}").unwrap();
    writeln!(out, "    .certo-view {{ padding: 1rem; }}").unwrap();
    writeln!(out, "  </style>").unwrap();
    writeln!(out, "</head>").unwrap();
    writeln!(out, "<body>").unwrap();
    write!(out, "{}", live_comment).unwrap();
    writeln!(out, "<div id=\"{}-view\" class=\"certo-view\"", slug).unwrap();
    writeln!(out, "     hx-get=\"{}\" hx-trigger=\"load\" hx-swap=\"innerHTML\">", endpoint).unwrap();
    write!(out, "{}", layout_html).unwrap();
    writeln!(out, "</div>").unwrap();
    writeln!(out, "</body>").unwrap();
    writeln!(out, "</html>").unwrap();
    out
}

/// Extract the value of `layout = <expr>` from a view's parsed block body.
///
/// The parser stores `view Foo { layout = ... }` as `Expr::Block` containing
/// `Stmt::Assign { target: "layout", value }`. We dig out that value here so
/// `layout_to_html` sees the actual UI primitive expression.
fn extract_layout_expr(block: &Expr) -> Option<&Expr> {
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

fn slugify(name: &str) -> String {
    let mut s = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            s.push('-');
        }
        s.push(c.to_lowercase().next().unwrap());
    }
    s
}

fn html_esc(s: &str) -> String { crate::layout::html_escape(s) }

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;
    use certo_ast::decl::Decl;

    fn view_html(src: &str) -> String {
        let m = parse(src).expect("parse");
        for sd in &m.decls {
            if let Decl::View(v) = &sd.node {
                return generate_view(v);
            }
        }
        panic!("no view");
    }

    #[test]
    fn is_valid_html_skeleton() {
        let src = "module M\nview Home { layout = Text(\"Hi\") }\n";
        let html = view_html(src);
        assert!(html.contains("<!DOCTYPE html>"), "missing doctype\n{}", html);
        assert!(html.contains("<title>Home</title>"), "missing title\n{}", html);
        assert!(html.contains("</html>"), "missing closing html\n{}", html);
    }

    #[test]
    fn htmx_script_included() {
        let src = "module M\nview Home { layout = Text(\"Hi\") }\n";
        let html = view_html(src);
        assert!(html.contains("htmx.org"), "missing htmx script\n{}", html);
    }

    #[test]
    fn hx_get_uses_slug() {
        let src = "module M\nview UserProfile { layout = Text(\"Hi\") }\n";
        let html = view_html(src);
        assert!(html.contains("hx-get=\"/user-profile\""), "wrong slug\n{}", html);
        assert!(html.contains("id=\"user-profile-view\""), "wrong id\n{}", html);
    }

    #[test]
    fn layout_content_appears_in_body() {
        let src = "module M\nview Foo { layout = Text(\"hello world\") }\n";
        let html = view_html(src);
        assert!(html.contains("<p>hello world</p>"), "missing layout\n{}", html);
    }

    #[test]
    fn slugify_pascal_case() {
        assert_eq!(super::slugify("UserProfile"), "user-profile");
        assert_eq!(super::slugify("Dashboard"),   "dashboard");
        assert_eq!(super::slugify("MyBigView"),   "my-big-view");
    }
}
