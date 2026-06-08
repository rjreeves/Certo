//! Generate a standalone HTML page from a `FormDecl`.
//!
//! # Example
//!
//! ```certo
//! form CreateUser {
//!   target = User
//!   fields {
//!     name        label = "Full Name"   placeholder = "Jane Smith"
//!     email       label = "Email"       placeholder = "jane@example.com"
//!     bio         label = "Bio"         rows = 4
//!   }
//!   on_submit = save
//!   on_success = redirect("/users")
//! }
//! ```
//!
//! Generates `create-user.html`:
//!
//! ```html
//! <!DOCTYPE html>
//! …
//! <form id="create-user-form"
//!       hx-post="/user"
//!       hx-swap="outerHTML">
//!   <div class="field">
//!     <label for="name">Full Name</label>
//!     <input id="name" name="name" type="text" placeholder="Jane Smith" required>
//!   </div>
//!   …
//!   <button type="submit">Submit</button>
//! </form>
//! ```

use std::fmt::Write as _;
use certo_ast::decl::{FormDecl, FormField};
use crate::layout::html_escape;

/// Generate a full standalone HTML page for a single `FormDecl`.
pub fn generate_form(f: &FormDecl) -> String {
    let name   = &f.name.node;
    let slug   = slugify(name);
    let target = f.target.segments.last()
        .map(|s| slugify(&s.node))
        .unwrap_or_else(|| slug.clone());

    let mut out = String::new();

    writeln!(out, "<!DOCTYPE html>").unwrap();
    writeln!(out, "<html lang=\"en\">").unwrap();
    writeln!(out, "<head>").unwrap();
    writeln!(out, "  <meta charset=\"UTF-8\">").unwrap();
    writeln!(out, "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">").unwrap();
    writeln!(out, "  <title>{}</title>", html_escape(name)).unwrap();
    writeln!(out, "  <script src=\"https://unpkg.com/htmx.org@2\" defer></script>").unwrap();
    writeln!(out, "  <style>").unwrap();
    writeln!(out, "    .field {{ display: flex; flex-direction: column; gap: 0.25rem; margin-bottom: 1rem; }}").unwrap();
    writeln!(out, "    label {{ font-weight: bold; }}").unwrap();
    writeln!(out, "    input, textarea, select {{ padding: 0.4rem; border: 1px solid #ccc; border-radius: 4px; }}").unwrap();
    writeln!(out, "    .certo-form {{ max-width: 480px; padding: 1rem; }}").unwrap();
    writeln!(out, "  </style>").unwrap();
    writeln!(out, "</head>").unwrap();
    writeln!(out, "<body>").unwrap();

    // on_submit comment (user fills in endpoint)
    writeln!(out, "<!-- TODO: on_submit handler — set hx-post to your endpoint or use a server route -->").unwrap();

    writeln!(out, "<form id=\"{}-form\" class=\"certo-form\"", slug).unwrap();
    writeln!(out, "      hx-post=\"/{}\"", target).unwrap();
    writeln!(out, "      hx-swap=\"outerHTML\">").unwrap();

    for field in &f.fields {
        write!(out, "{}", render_field(field)).unwrap();
    }

    // on_success comment
    if f.on_success.is_some() {
        writeln!(out, "  <!-- TODO: on_success — add hx-on::after-request or redirect logic -->").unwrap();
    }

    writeln!(out, "  <button type=\"submit\">Submit</button>").unwrap();
    writeln!(out, "</form>").unwrap();
    writeln!(out, "</body>").unwrap();
    writeln!(out, "</html>").unwrap();
    out
}

// ------------------------------------------------------------------ //
// Field rendering
// ------------------------------------------------------------------ //

fn render_field(f: &FormField) -> String {
    let name        = &f.name.node;
    let label       = f.label.as_deref().unwrap_or(name.as_str());
    let placeholder = f.placeholder.as_deref().unwrap_or("");
    let rows        = f.rows;

    // Determine field type: textarea for rows > 1, else input
    let input_type = infer_input_type(name, &f.field_type);

    let mut out = String::new();
    writeln!(out, "  <div class=\"field\">").unwrap();
    writeln!(out, "    <label for=\"{}\">{}</label>", html_escape(name), html_escape(label)).unwrap();

    if rows.is_some_and(|r| r > 1) {
        let r = rows.unwrap();
        if placeholder.is_empty() {
            writeln!(out, "    <textarea id=\"{}\" name=\"{}\" rows=\"{}\"></textarea>",
                html_escape(name), html_escape(name), r).unwrap();
        } else {
            writeln!(out, "    <textarea id=\"{}\" name=\"{}\" rows=\"{}\" placeholder=\"{}\"></textarea>",
                html_escape(name), html_escape(name), r, html_escape(placeholder)).unwrap();
        }
    } else if input_type == "select" {
        writeln!(out, "    <!-- TODO: populate <select> options for '{}' -->", html_escape(name)).unwrap();
        writeln!(out, "    <select id=\"{}\" name=\"{}\"></select>",
            html_escape(name), html_escape(name)).unwrap();
    } else {
        let ph_attr = if placeholder.is_empty() {
            String::new()
        } else {
            format!(" placeholder=\"{}\"", html_escape(placeholder))
        };
        writeln!(out, "    <input id=\"{}\" name=\"{}\" type=\"{}\"{} required>",
            html_escape(name), html_escape(name), input_type, ph_attr).unwrap();
    }

    writeln!(out, "  </div>").unwrap();
    out
}

/// Infer the HTML input type from the field name or explicit `field_type` expression.
fn infer_input_type(name: &str, field_type: &Option<certo_ast::span::S<certo_ast::expr::Expr>>) -> &'static str {
    // Check explicit field_type expression first (only string/var literals)
    if let Some(ft) = field_type {
        use certo_ast::expr::{Expr, Lit};
        match &ft.node {
            Expr::Lit { value: Lit::String(s), .. } => return str_to_input_type(s),
            Expr::Path { path, .. } => {
                if let Some(seg) = path.segments.last() {
                    return str_to_input_type(&seg.node);
                }
            }
            _ => {}
        }
    }
    // Heuristic from field name
    let lower = name.to_lowercase();
    if lower.contains("email")    { return "email"; }
    if lower.contains("password") { return "password"; }
    if lower.contains("phone") || lower.contains("tel") { return "tel"; }
    if lower.contains("url")  || lower.contains("website") { return "url"; }
    if lower.contains("date") || lower.contains("birthday") { return "date"; }
    if lower.contains("time") { return "time"; }
    if lower.contains("number") || lower.contains("amount") || lower.contains("quantity") { return "number"; }
    if lower.contains("color") || lower.contains("colour") { return "color"; }
    "text"
}

fn str_to_input_type(s: &str) -> &'static str {
    match s.to_lowercase().as_str() {
        "email"    => "email",
        "password" => "password",
        "phone" | "tel" => "tel",
        "url"      => "url",
        "date"     => "date",
        "time"     => "time",
        "number"   => "number",
        "color" | "colour" => "color",
        "select" | "dropdown" => "select",
        "checkbox" => "checkbox",
        "range"    => "range",
        _          => "text",
    }
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

// ------------------------------------------------------------------ //
// Tests
// ------------------------------------------------------------------ //

#[cfg(test)]
mod tests {
    use super::*;
    use certo_ast::decl::{FormDecl, FormField};
    use certo_ast::types::ModulePath;
    use certo_ast::span::{S, Span};

    fn dummy_span() -> Span { Span { start: 0, end: 0 } }
    fn ident(s: &str) -> S<String> { S::new(s.to_owned(), dummy_span()) }

    fn path(segments: &[&str]) -> ModulePath {
        ModulePath {
            segments: segments.iter().map(|s| ident(s)).collect(),
            span: dummy_span(),
        }
    }

    fn simple_field(name: &str) -> FormField {
        FormField {
            name: ident(name),
            label: None, placeholder: None,
            field_type: None, options: None, rows: None,
            span: dummy_span(),
        }
    }

    fn labeled_field(name: &str, label: &str, placeholder: Option<&str>) -> FormField {
        FormField {
            name: ident(name),
            label: Some(label.to_owned()),
            placeholder: placeholder.map(str::to_owned),
            field_type: None, options: None, rows: None,
            span: dummy_span(),
        }
    }

    fn textarea_field(name: &str, label: &str, rows: u32) -> FormField {
        FormField {
            name: ident(name),
            label: Some(label.to_owned()),
            placeholder: None,
            field_type: None, options: None,
            rows: Some(rows),
            span: dummy_span(),
        }
    }

    fn make_form(name: &str, target: &str, fields: Vec<FormField>, on_submit: bool) -> FormDecl {
        FormDecl {
            name: ident(name),
            target: path(&[target]),
            fields,
            on_submit:  if on_submit { Some(S::new(certo_ast::expr::Expr::Lit {
                value: certo_ast::expr::Lit::Unit,
                span: dummy_span(),
            }, dummy_span())) } else { None },
            on_success: None,
            span: dummy_span(),
        }
    }

    #[test]
    fn basic_form_structure() {
        let f = make_form("CreateUser", "User", vec![simple_field("name")], false);
        let html = generate_form(&f);
        assert!(html.contains("<!DOCTYPE html>"), "missing doctype\n{}", html);
        assert!(html.contains("<form "),          "missing form\n{}", html);
        assert!(html.contains("</form>"),         "missing closing form\n{}", html);
    }

    #[test]
    fn htmx_post_uses_target_slug() {
        let f = make_form("CreateUser", "User", vec![simple_field("name")], false);
        let html = generate_form(&f);
        assert!(html.contains("hx-post=\"/user\""), "wrong hx-post\n{}", html);
    }

    #[test]
    fn field_with_label_and_placeholder() {
        let f = make_form("F", "T", vec![
            labeled_field("email", "Email", Some("you@example.com"))
        ], false);
        let html = generate_form(&f);
        assert!(html.contains("<label for=\"email\">Email</label>"), "missing label\n{}", html);
        assert!(html.contains("placeholder=\"you@example.com\""),   "missing placeholder\n{}", html);
    }

    #[test]
    fn textarea_for_rows() {
        let f = make_form("F", "T", vec![textarea_field("bio", "Bio", 4)], false);
        let html = generate_form(&f);
        assert!(html.contains("<textarea"),  "missing textarea\n{}", html);
        assert!(html.contains("rows=\"4\""), "wrong rows\n{}", html);
    }

    #[test]
    fn email_field_inferred_from_name() {
        let f = make_form("F", "T", vec![simple_field("email")], false);
        let html = generate_form(&f);
        assert!(html.contains("type=\"email\""), "wrong input type\n{}", html);
    }

    #[test]
    fn password_field_inferred() {
        let f = make_form("F", "T", vec![simple_field("password")], false);
        let html = generate_form(&f);
        assert!(html.contains("type=\"password\""), "wrong input type\n{}", html);
    }

    #[test]
    fn submit_button_present() {
        let f = make_form("F", "T", vec![simple_field("name")], false);
        let html = generate_form(&f);
        assert!(html.contains("<button type=\"submit\">Submit</button>"), "missing submit\n{}", html);
    }

    #[test]
    fn on_submit_todo_comment_present() {
        let f = make_form("F", "T", vec![simple_field("name")], true);
        let html = generate_form(&f);
        assert!(html.contains("TODO: on_submit"), "missing TODO comment\n{}", html);
    }

    #[test]
    fn form_id_uses_slug() {
        let f = make_form("CreateUser", "User", vec![simple_field("name")], false);
        let html = generate_form(&f);
        assert!(html.contains("id=\"create-user-form\""), "wrong form id\n{}", html);
    }
}
