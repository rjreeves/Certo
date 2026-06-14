//! `certo-ui` — compile Certo `view` and `form` declarations to Htmx HTML pages.
//!
//! # Pipeline
//!
//! ```text
//! .cto source
//!     │
//!     ▼ certo_parser::parse()
//!     │ Module
//!     ▼ ui::emit_module()
//!     │ Vec<(filename, html)>
//!     ▼ write to disk
//!     │ view-name.html, form-name.html
//! ```
//!
//! # Supported layout primitives
//!
//! `VStack`, `HStack`, `Text`, `Heading`, `Button`, `Link`, `Image`, `Input`,
//! `For` (comment stub), `If` (comment stub).
//! Unknown calls emit an HTML comment rather than failing.

pub mod error;
pub mod layout;
pub mod view;
pub mod form;
pub mod server;

use certo_ast::decl::Decl;
use certo_ast::module::Module;

pub use error::UiError;
pub use view::generate_view;
pub use form::generate_form;
pub use server::emit_server;

/// Compile all `view` and `form` declarations to a single runnable `server.cto`.
///
/// This is the default `certo-ui` mode. Use `emit_html` for the legacy
/// per-page HTML output.
pub fn emit_module(module: &Module) -> Result<Vec<(String, String)>, UiError> {
    let code = emit_server(module)?;
    Ok(vec![("server.cto".to_string(), code)])
}

/// Compile all `view` and `form` declarations to individual HTML files (legacy mode).
///
/// Activated by `certo-ui --html <file.cto>`.
pub fn emit_html(module: &Module) -> Result<Vec<(String, String)>, UiError> {
    let mut out = Vec::new();

    for sd in &module.decls {
        match &sd.node {
            Decl::View(v) => {
                let slug     = slugify(&v.name.node);
                let filename = format!("{}.html", slug);
                let html     = generate_view(v);
                out.push((filename, html));
            }
            Decl::Form(f) => {
                let slug     = slugify(&f.name.node);
                let filename = format!("{}.html", slug);
                let html     = generate_form(f);
                out.push((filename, html));
            }
            _ => {}
        }
    }

    if out.is_empty() {
        Err(UiError::NoDeclarations)
    } else {
        Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;
    use certo_parser::parse;

    #[test]
    fn emit_module_single_view() {
        let src = "module M\nview Home { layout = Text(\"Hi\") }\n";
        let files = emit_module(&parse(src).unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.iter().any(|(f, _)| f == "home.html"));
    }

    #[test]
    fn emit_module_no_ui_gives_error() {
        let src = "module M\npub fn add(a: Int, b: Int): Int = a + b\n";
        let err = emit_module(&parse(src).unwrap()).unwrap_err();
        assert!(matches!(err, UiError::NoDeclarations));
    }
}
