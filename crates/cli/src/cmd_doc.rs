use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

pub fn cmd_doc(args: &[String]) {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut serve = false;
    let mut port: u16 = 4000;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| { eprintln!("error: -o requires a path"); process::exit(2); })
                ));
            }
            "--serve" => serve = true,
            "--port" => {
                i += 1;
                let raw = args.get(i).unwrap_or_else(|| { eprintln!("error: --port requires a number"); process::exit(2); });
                port = raw.parse().unwrap_or_else(|_| {
                    eprintln!("error: --port must be a number between 1 and 65535, got {:?}", raw);
                    process::exit(2);
                });
            }
            "--help" | "-h" => {
                println!("Usage: certo doc <file.cto> [-o <dir>] [--serve] [--port <n>]");
                println!();
                println!("Generate HTML documentation from /// doc comments.");
                println!("Output defaults to docs/ next to the source file.");
                println!();
                println!("Options:");
                println!("  --serve       Serve the generated docs over HTTP after generating them");
                println!("  --port <n>    Port to serve on (default: 4000, requires --serve)");
                return;
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {}", other);
                process::exit(2);
            }
            path => {
                if input.is_some() { eprintln!("error: only one input file supported"); process::exit(2); }
                input = Some(PathBuf::from(path));
            }
        }
        i += 1;
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("error: no input file");
        eprintln!("usage: certo doc <file.cto>");
        process::exit(2);
    });

    let src = std::fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", input.display(), e);
        process::exit(1);
    });

    let out_dir = out_dir.unwrap_or_else(|| {
        input.parent().unwrap_or(Path::new(".")).join("docs")
    });
    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| {
        eprintln!("error: cannot create {}: {}", out_dir.display(), e);
        process::exit(1);
    });

    let module_name = input.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module")
        .to_string();

    let items = extract_doc_items(&src);
    let html = render_html(&module_name, &items, &input.display().to_string());

    let out_path = out_dir.join("index.html");
    std::fs::write(&out_path, html).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", out_path.display(), e);
        process::exit(1);
    });

    println!("docs → {}", out_path.display());

    if serve {
        println!("Serving docs at http://localhost:{}/ (Ctrl+C to stop)", port);
        if let Err(e) = crate::static_serve::serve_dir(&out_dir, port) {
            eprintln!("error: could not serve on port {}: {}", port, e);
            process::exit(1);
        }
    }
}

// ------------------------------------------------------------------ //
// Doc extraction
// ------------------------------------------------------------------ //

#[derive(Debug, Clone)]
pub struct DocItem {
    pub kind:      ItemKind,
    pub name:      String,
    pub signature: String,
    pub doc:       String,
    pub anchor:    String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Function,
    Type,
    Val,
    Var,
    // BACKLOG item 222 — validator awareness. Same crude, line-based
    // recognition already used for `fn`/`type`/`val`/`var` above (this
    // scanner never parses a real AST); `Rule` is a *nested* declaration
    // (inside a `validator { ... }` body) but gets picked up the same
    // way `fn`/`type` already are regardless of nesting, matching the
    // spec's own promise ("certo docs produces a rule catalogue from your
    // validator declarations," docs/section-16-validators.md §16.11).
    Validator,
    Rule,
    Constraint,
    // BACKLOG item 319 — §16.14's "New Top-Level Declarations" table lists
    // four kinds (`constraint`, `temporal`, `validator`, `rule`); item 222
    // only ever added the first, third, and fourth. Same crude recognition.
    Temporal,
}

impl ItemKind {
    fn label(&self) -> &'static str {
        match self {
            ItemKind::Function   => "fn",
            ItemKind::Type       => "type",
            ItemKind::Val        => "val",
            ItemKind::Var        => "var",
            ItemKind::Validator  => "validator",
            ItemKind::Rule       => "rule",
            ItemKind::Constraint => "constraint",
            ItemKind::Temporal   => "temporal",
        }
    }
}

/// Scan source lines for `///` comment blocks immediately before declarations.
pub fn extract_doc_items(src: &str) -> Vec<DocItem> {
    let mut items: Vec<DocItem> = Vec::new();
    let mut doc_buf: Vec<String> = Vec::new();
    // track anchor uniqueness
    let mut seen: HashMap<String, usize> = HashMap::new();

    for line in src.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("///") {
            // Doc comment line — strip leading space
            let text = rest.strip_prefix(' ').unwrap_or(rest);
            doc_buf.push(text.to_string());
            continue;
        }

        // Blank lines between doc and decl are allowed
        if trimmed.is_empty() && !doc_buf.is_empty() {
            continue;
        }

        if let Some(item) = try_parse_decl_line(trimmed) {
            let doc = doc_buf.drain(..).collect::<Vec<_>>().join("\n");
            if !doc.is_empty() || should_always_include(&item.kind) {
                // Build a unique anchor
                let base = item.name.clone();
                let count = seen.entry(base.clone()).or_insert(0);
                let anchor = if *count == 0 {
                    base.clone()
                } else {
                    format!("{}-{}", base, count)
                };
                *count += 1;
                items.push(DocItem { anchor, doc, ..item });
            }
        } else {
            // Non-doc, non-blank, non-decl line — reset accumulator
            doc_buf.clear();
        }
    }

    items
}

fn should_always_include(kind: &ItemKind) -> bool {
    matches!(kind, ItemKind::Function | ItemKind::Type | ItemKind::Validator | ItemKind::Rule | ItemKind::Constraint | ItemKind::Temporal)
}

/// Try to parse a declaration line and return a partial DocItem (no doc/anchor yet).
fn try_parse_decl_line(trimmed: &str) -> Option<DocItem> {
    // Strip visibility / async modifiers
    let s = trimmed
        .strip_prefix("pub ")
        .unwrap_or(trimmed);
    let s = s.strip_prefix("async ").unwrap_or(s);
    let s = s.strip_prefix("extern ").unwrap_or(s);

    if s.starts_with("fn ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "fn ")?;
        // Strip generic params from name: `foo<T>` → `foo`
        let name = name.split(['<', '(']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Function, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    if s.starts_with("type ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "type ")?;
        let name = name.split(['<', '=']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Type, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    if s.starts_with("val ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "val ")?;
        let name = name.split([':']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Val, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    if s.starts_with("var ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "var ")?;
        let name = name.split([':']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Var, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    // BACKLOG item 222 — validator awareness.
    if s.starts_with("validator ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "validator ")?;
        let name = name.trim().to_string();
        return Some(DocItem { kind: ItemKind::Validator, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    if s.starts_with("constraint ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "constraint ")?;
        let name = name.split(['=']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Constraint, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    // BACKLOG item 319 — `temporal Name = Duration...` has the identical
    // shape to `constraint Name = expr`, same extraction.
    if s.starts_with("temporal ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "temporal ")?;
        let name = name.split(['=']).next()?.trim().to_string();
        return Some(DocItem { kind: ItemKind::Temporal, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    // `rule name {` — nested inside a `validator { ... }` body; this
    // scanner has no notion of nesting for `fn`/`type` either, so a rule
    // is picked up the same crude, line-based way.
    if s.starts_with("rule ") {
        let sig = trimmed.to_string();
        let name = extract_word_after(s, "rule ")?;
        let name = name.trim().to_string();
        return Some(DocItem { kind: ItemKind::Rule, name, signature: sig, doc: String::new(), anchor: String::new() });
    }

    None
}

fn extract_word_after<'a>(haystack: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = haystack.strip_prefix(prefix)?;
    Some(rest.split_whitespace().next().unwrap_or(""))
}

// ------------------------------------------------------------------ //
// HTML rendering
// ------------------------------------------------------------------ //

pub fn render_html(module_name: &str, items: &[DocItem], source_path: &str) -> String {
    let sidebar = render_sidebar(items);
    let content = render_content(items);

    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{module_name} — Certo Docs</title>
<style>
*, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  font-size: 15px;
  color: #1a1a1a;
  background: #fafafa;
  display: flex;
  min-height: 100vh;
}}
nav {{
  width: 240px;
  flex-shrink: 0;
  background: #fff;
  border-right: 1px solid #e5e7eb;
  padding: 24px 0;
  position: sticky;
  top: 0;
  height: 100vh;
  overflow-y: auto;
}}
nav h2 {{
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: .08em;
  color: #9ca3af;
  padding: 0 20px 8px;
}}
nav ul {{ list-style: none; }}
nav ul li a {{
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 20px;
  text-decoration: none;
  color: #374151;
  font-size: 13.5px;
  border-left: 2px solid transparent;
}}
nav ul li a:hover {{
  background: #f3f4f6;
  color: #111;
  border-left-color: #6366f1;
}}
nav .badge {{
  font-size: 10px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 3px;
  font-family: "SF Mono", "Fira Code", monospace;
}}
nav .badge-fn   {{ background: #ede9fe; color: #7c3aed; }}
nav .badge-type {{ background: #fef3c7; color: #b45309; }}
nav .badge-val  {{ background: #d1fae5; color: #065f46; }}
nav .badge-var  {{ background: #fce7f3; color: #9d174d; }}
nav .badge-validator  {{ background: #dbeafe; color: #1d4ed8; }}
nav .badge-rule       {{ background: #e0e7ff; color: #4338ca; }}
nav .badge-constraint {{ background: #fee2e2; color: #b91c1c; }}
nav .badge-temporal   {{ background: #d1fae5; color: #047857; }}
main {{
  flex: 1;
  padding: 40px 60px;
  max-width: 900px;
}}
header {{
  border-bottom: 1px solid #e5e7eb;
  padding-bottom: 20px;
  margin-bottom: 40px;
}}
header h1 {{ font-size: 26px; font-weight: 700; color: #111; }}
header .source {{ font-size: 12px; color: #6b7280; margin-top: 4px; font-family: monospace; }}
.item {{
  margin-bottom: 44px;
  padding-bottom: 44px;
  border-bottom: 1px solid #f0f0f0;
}}
.item:last-child {{ border-bottom: none; }}
.item-header {{
  display: flex;
  align-items: baseline;
  gap: 10px;
  margin-bottom: 10px;
}}
.item-header h3 {{
  font-size: 16px;
  font-weight: 600;
  color: #111;
}}
.item pre {{
  background: #f8f8f8;
  border: 1px solid #e5e7eb;
  border-left: 3px solid #6366f1;
  border-radius: 4px;
  padding: 12px 16px;
  font-family: "SF Mono", "Fira Code", "Cascadia Code", monospace;
  font-size: 13px;
  overflow-x: auto;
  margin-bottom: 12px;
  white-space: pre-wrap;
  word-break: break-word;
}}
.item-doc {{
  color: #374151;
  line-height: 1.65;
  font-size: 14.5px;
}}
.item-doc p {{ margin-bottom: 8px; }}
.item-doc code {{
  background: #f3f4f6;
  padding: 1px 5px;
  border-radius: 3px;
  font-family: "SF Mono", "Fira Code", monospace;
  font-size: 12.5px;
}}
.no-doc {{ color: #9ca3af; font-style: italic; font-size: 13px; }}
footer {{
  margin-top: 60px;
  padding-top: 20px;
  border-top: 1px solid #e5e7eb;
  color: #9ca3af;
  font-size: 12px;
}}
</style>
</head>
<body>
<nav>
  <h2>{module_name}</h2>
  <ul>
{sidebar}
  </ul>
</nav>
<main>
  <header>
    <h1>{module_name}</h1>
    <div class="source">{source_path}</div>
  </header>
{content}
  <footer>Generated by <strong>certo doc</strong></footer>
</main>
</body>
</html>"#)
}

fn render_sidebar(items: &[DocItem]) -> String {
    items.iter().map(|item| {
        let badge_class = match item.kind {
            ItemKind::Function   => "badge-fn",
            ItemKind::Type       => "badge-type",
            ItemKind::Val        => "badge-val",
            ItemKind::Var        => "badge-var",
            ItemKind::Validator  => "badge-validator",
            ItemKind::Rule       => "badge-rule",
            ItemKind::Constraint => "badge-constraint",
            ItemKind::Temporal   => "badge-temporal",
        };
        format!(
            "    <li><a href=\"#{anchor}\"><span class=\"badge {badge_class}\">{kind}</span>{name}</a></li>",
            anchor = html_escape(&item.anchor),
            badge_class = badge_class,
            kind = item.kind.label(),
            name = html_escape(&item.name),
        )
    }).collect::<Vec<_>>().join("\n")
}

fn render_content(items: &[DocItem]) -> String {
    items.iter().map(|item| {
        let badge_class = match item.kind {
            ItemKind::Function   => "badge-fn",
            ItemKind::Type       => "badge-type",
            ItemKind::Val        => "badge-val",
            ItemKind::Var        => "badge-var",
            ItemKind::Validator  => "badge-validator",
            ItemKind::Rule       => "badge-rule",
            ItemKind::Constraint => "badge-constraint",
            ItemKind::Temporal   => "badge-temporal",
        };
        let doc_html = if item.doc.is_empty() {
            r#"<p class="no-doc">No documentation.</p>"#.to_string()
        } else {
            render_doc_markdown(&item.doc)
        };
        format!(
            "  <div class=\"item\" id=\"{anchor}\">\n    <div class=\"item-header\">\n      \
             <span class=\"badge {badge_class}\">{kind}</span>\n      <h3>{name}</h3>\n    </div>\n    \
             <pre>{sig}</pre>\n    <div class=\"item-doc\">{doc_html}</div>\n  </div>",
            anchor     = html_escape(&item.anchor),
            badge_class = badge_class,
            kind       = item.kind.label(),
            name       = html_escape(&item.name),
            sig        = html_escape(&item.signature),
            doc_html   = doc_html,
        )
    }).collect::<Vec<_>>().join("\n")
}

/// Minimal doc comment renderer: blank lines → paragraph breaks, backticks → <code>.
fn render_doc_markdown(doc: &str) -> String {
    let mut out = String::new();
    let mut para = String::new();

    let flush_para = |para: &mut String, out: &mut String| {
        if !para.trim().is_empty() {
            out.push_str("<p>");
            out.push_str(&render_inline(para.trim()));
            out.push_str("</p>");
        }
        para.clear();
    };

    for line in doc.lines() {
        if line.trim().is_empty() {
            flush_para(&mut para, &mut out);
        } else {
            if !para.is_empty() { para.push(' '); }
            para.push_str(line.trim());
        }
    }
    flush_para(&mut para, &mut out);
    out
}

/// Inline rendering: `backtick` → <code>, & < > escaped.
fn render_inline(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '`' => {
                let mut inner = String::new();
                for c2 in chars.by_ref() {
                    if c2 == '`' { break; }
                    inner.push(c2);
                }
                out.push_str("<code>");
                out.push_str(&html_escape(&inner));
                out.push_str("</code>");
            }
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------ //
    // BACKLOG item 222 — `certo doc` validator/rule/constraint awareness.
    // ------------------------------------------------------------------ //

    #[test]
    fn validator_declaration_is_recognized() {
        let src = "validator OrderSubmit for Order errors OrderError {\n    rule active { require true else OrderError.X }\n}\n";
        let items = extract_doc_items(src);
        assert!(items.iter().any(|i| i.kind == ItemKind::Validator && i.name == "OrderSubmit"),
            "expected an OrderSubmit validator item, got: {:?}", items.iter().map(|i| (&i.kind, &i.name)).collect::<Vec<_>>());
    }

    #[test]
    fn rule_declaration_nested_in_a_validator_is_recognized() {
        let src = "validator OrderSubmit for Order errors OrderError {\n    rule customer_active { require true else OrderError.X }\n}\n";
        let items = extract_doc_items(src);
        assert!(items.iter().any(|i| i.kind == ItemKind::Rule && i.name == "customer_active"),
            "expected a customer_active rule item, got: {:?}", items.iter().map(|i| (&i.kind, &i.name)).collect::<Vec<_>>());
    }

    #[test]
    fn constraint_declaration_is_recognized() {
        let src = "constraint UserIsAdmin = user.role == Admin\n";
        let items = extract_doc_items(src);
        assert!(items.iter().any(|i| i.kind == ItemKind::Constraint && i.name == "UserIsAdmin"),
            "expected a UserIsAdmin constraint item, got: {:?}", items.iter().map(|i| (&i.kind, &i.name)).collect::<Vec<_>>());
    }

    // BACKLOG item 319 — §16.14's own "New Top-Level Declarations" table
    // lists `temporal` alongside `constraint`/`validator`/`rule`; item 222
    // never added a matching branch for it.
    #[test]
    fn temporal_declaration_is_recognized() {
        let src = "temporal VoidWindow = Duration.days(30)\n";
        let items = extract_doc_items(src);
        assert!(items.iter().any(|i| i.kind == ItemKind::Temporal && i.name == "VoidWindow"),
            "expected a VoidWindow temporal item, got: {:?}", items.iter().map(|i| (&i.kind, &i.name)).collect::<Vec<_>>());
    }

    #[test]
    fn temporal_is_always_included_without_a_doc_comment() {
        let src = "temporal VoidWindow = Duration.days(30)\n";
        let items = extract_doc_items(src);
        assert!(items.iter().any(|i| i.kind == ItemKind::Temporal), "expected a temporal item without any /// doc comment");
    }

    #[test]
    fn validator_rule_constraint_are_always_included_without_a_doc_comment() {
        // Matches fn/type's own existing convention — a rule catalogue is
        // useful even undocumented.
        let src = "constraint Foo = true\nvalidator V for Order errors E {\n    rule r { require true else E.X }\n}\n";
        let items = extract_doc_items(src);
        assert_eq!(items.len(), 3, "expected all 3 items included with no doc comments, got: {:?}",
            items.iter().map(|i| (&i.kind, &i.name)).collect::<Vec<_>>());
    }

    #[test]
    fn documented_validator_keeps_its_doc_comment() {
        let src = "/// Validates a submitted order.\nvalidator OrderSubmit for Order errors OrderError {\n}\n";
        let items = extract_doc_items(src);
        let v = items.iter().find(|i| i.kind == ItemKind::Validator).expect("expected a validator item");
        assert_eq!(v.doc, "Validates a submitted order.");
    }
}
