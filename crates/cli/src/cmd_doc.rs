use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

pub fn cmd_doc(args: &[String]) {
    let mut input: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(
                    args.get(i).unwrap_or_else(|| { eprintln!("error: -o requires a path"); process::exit(2); })
                ));
            }
            "--help" | "-h" => {
                println!("Usage: certo doc <file.cto> [-o <dir>]");
                println!();
                println!("Generate HTML documentation from /// doc comments.");
                println!("Output defaults to docs/ next to the source file.");
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
}

impl ItemKind {
    fn label(&self) -> &'static str {
        match self {
            ItemKind::Function => "fn",
            ItemKind::Type     => "type",
            ItemKind::Val      => "val",
            ItemKind::Var      => "var",
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
    matches!(kind, ItemKind::Function | ItemKind::Type)
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
            ItemKind::Function => "badge-fn",
            ItemKind::Type     => "badge-type",
            ItemKind::Val      => "badge-val",
            ItemKind::Var      => "badge-var",
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
            ItemKind::Function => "badge-fn",
            ItemKind::Type     => "badge-type",
            ItemKind::Val      => "badge-val",
            ItemKind::Var      => "badge-var",
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
