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
}
