/// Interactive REPL for the Certo language.
///
/// Each iteration synthesises a complete `.cto` source file from the
/// accumulated session declarations plus the user's new input, runs the full
/// compile-to-C pipeline, and executes the result in a temp directory.

use std::io::{self, Write};
use certo_typeck::{TypeError, TypeErrorKind, TypeEnv, Ty, assign_var_names};
use certo_diagnostics::{Diagnostic, render_all};

use crate::CERTO_VERSION;

// ------------------------------------------------------------------ //
// Entry point
// ------------------------------------------------------------------ //

pub fn cmd_repl() {
    println!("Certo v{} (c) SyntrA 2026", CERTO_VERSION);
    println!("Type :help for commands, :quit or . to exit.");
    println!();

    let mut session: Vec<String> = Vec::new();
    let mut buf     = String::new();
    let colour      = crate::stderr_is_tty();

    loop {
        let prompt = if buf.is_empty() { ">>> " } else { "... " };
        print!("{}", prompt);
        io::stdout().flush().ok();

        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,          // EOF / Ctrl-D
            Ok(_) => {}
            Err(_) => break,
        }

        // Strip trailing newline only; preserve indentation inside blocks.
        let raw = line.trim_end_matches('\n').trim_end_matches('\r');

        // REPL meta-commands — only at the start of a fresh input.
        if buf.is_empty() {
            if handle_command(raw, &mut session) {
                continue;
            }
        }

        buf.push_str(raw);
        buf.push('\n');

        // Wait for more input if brackets are unbalanced.
        if needs_continuation(&buf) {
            continue;
        }

        let input = std::mem::take(&mut buf);
        let input = input.trim().to_string();
        if input.is_empty() {
            continue;
        }

        repl_eval(&input, &mut session, colour);
    }

    println!();
    println!("Bye.");
}

// ------------------------------------------------------------------ //
// Meta-commands
// ------------------------------------------------------------------ //

/// Returns true if the line was a REPL command (consumed).
fn handle_command(line: &str, session: &mut Vec<String>) -> bool {
    match line {
        ":quit" | ":q" | ":exit" | "." => {
            println!();
            println!("Bye.");
            std::process::exit(0);
        }
        ":help" | ":h" => {
            print_repl_help();
            return true;
        }
        ":clear" | ":reset" => {
            session.clear();
            println!("Session cleared.");
            return true;
        }
        ":session" | ":s" => {
            if session.is_empty() {
                println!("(empty session)");
            } else {
                println!("{}", session.join("\n\n"));
            }
            return true;
        }
        s if s.starts_with(":type ") => {
            let expr = s["type ".len() + 1..].trim();
            repl_show_type(expr, session);
            return true;
        }
        s if s.starts_with(':') => {
            eprintln!("Unknown REPL command: {}  (try :help)", s);
            return true;
        }
        _ => false,
    }
}

fn print_repl_help() {
    println!("REPL commands:");
    println!("  :help, :h          This message");
    println!("  :quit, :q, .       Exit");
    println!("  :clear             Clear the session (forget all definitions)");
    println!("  :session, :s       Show accumulated session declarations");
    println!("  :type <expr>       Show the inferred type of an expression");
    println!();
    println!("Input hints:");
    println!("  fn foo(x: Int): Int = x + 1   -- add a function to the session");
    println!("  val pi = 3.14159              -- add a top-level constant");
    println!("  print(intToText(1 + 2))       -- execute a statement");
    println!("  1 + 2                         -- bare expressions are auto-printed");
    println!("  {{ val x = 10; print(intToText(x)) }}  -- multi-statement block");
}

// ------------------------------------------------------------------ //
// Eval loop
// ------------------------------------------------------------------ //

fn repl_eval(input: &str, session: &mut Vec<String>, colour: bool) {
    if looks_like_decl(input) {
        eval_decl(input, session, colour);
    } else {
        eval_expr_or_stmt(input, session, colour);
    }
}

/// Decide whether the user is entering a top-level declaration.
fn looks_like_decl(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with("fn ")   ||
    t.starts_with("pub fn")  ||
    t.starts_with("type ")  ||
    t.starts_with("import ") ||
    // val/var at top level (no leading indent) — not inside a block
    (t.starts_with("val ") && !s.starts_with(' ') && !s.starts_with('\t')) ||
    (t.starts_with("var ") && !s.starts_with(' ') && !s.starts_with('\t'))
}

// ------------------------------------------------------------------ //
// Declaration eval
// ------------------------------------------------------------------ //

fn eval_decl(input: &str, session: &mut Vec<String>, colour: bool) {
    // Parse the declaration in the context of the full session.
    let test_src = make_module_src(session, None);
    // Append the new decl and try to parse the combined module.
    let combined = format!("{}\n{}\n", test_src, input);
    match certo_parser::parse(&combined) {
        Err(errs) => {
            for e in &errs {
                eprintln!("parse error: {}", e);
            }
            return;
        }
        Ok(module) => {
            // Type-check the combined module.
            let mut env     = TypeEnv::new();
            let mut counter = 0u32;
            env.seed_builtins(&mut counter);
            certo_stdlib::seed_stdlib(&mut env, &mut counter);
            if let Err(errs) = certo_typeck::check_module_seeded(&module, env, counter) {
                let diags: Vec<Diagnostic> = errs.iter()
                    .map(crate::type_error_to_diagnostic)
                    .collect();
                eprint!("{}", render_all(&diags, &combined, "<repl>", colour));
                return;
            }
        }
    }
    session.push(input.to_string());
    // Print a short "defined" notice based on what was declared.
    let label = decl_label(input);
    println!("{}", label);
}

/// Return a short summary like "defined: foo" for display after a decl is accepted.
fn decl_label(input: &str) -> String {
    let t = input.trim();
    for prefix in &["pub fn ", "fn ", "type ", "val ", "var "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            // Name ends at the first whitespace, `(`, `:`, or `=`.
            let name = rest
                .split(|c: char| c.is_whitespace() || c == '(' || c == ':' || c == '=')
                .next()
                .unwrap_or("?");
            let kw = prefix.trim();
            return format!("defined: {} {}", kw, name);
        }
    }
    "defined.".to_string()
}

// ------------------------------------------------------------------ //
// Expression / statement eval
// ------------------------------------------------------------------ //

fn eval_expr_or_stmt(input: &str, session: &mut Vec<String>, colour: bool) {
    // Probe the type of the input as an expression first.
    // If it's a print-able primitive, auto-wrap it so the value is displayed.
    // Skip the probe when the input uses `?` — the probe context is Unit-returning
    // and would produce a spurious type error before we get a chance to run it.
    if !body_uses_try(input) {
        if let Some(ty) = infer_type(input, session) {
            if let Some(print_stmt) = auto_print_for(&ty, input) {
                let src2 = make_module_src(session, Some(&print_stmt));
                match try_compile_and_run(&src2, colour) {
                    EvalResult::Ok => return,
                    EvalResult::TypeError(_) => {} // fall through to plain run
                    EvalResult::ParseError(msg) => { eprintln!("parse error: {}", msg); return; }
                    EvalResult::CompileError => return,
                    EvalResult::RuntimeError(code) => { eprintln!("exited with code {}", code); return; }
                }
            }
        }
    }

    // Plain run — statement, Unit expression, complex type, or ?-containing expr.
    // make_module_src detects ? and switches to a Result-returning wrapper.
    let src = make_module_src(session, Some(input));
    match try_compile_and_run(&src, colour) {
        EvalResult::Ok => {}
        EvalResult::TypeError(errs) => {
            let diags: Vec<Diagnostic> = errs.iter()
                .map(crate::type_error_to_diagnostic)
                .collect();
            eprint!("{}", render_all(&diags, &src, "<repl>", colour));
        }
        EvalResult::ParseError(msg) => eprintln!("parse error: {}", msg),
        EvalResult::CompileError => {}
        EvalResult::RuntimeError(code) => eprintln!("exited with code {}", code),
    }
}

/// Build a print-statement that displays `expr` of the given type.
fn auto_print_for(ty: &Ty, expr: &str) -> Option<String> {
    match ty {
        Ty::Text  => Some(format!("print({})", expr)),
        Ty::Int | Ty::Int8 | Ty::Int16 | Ty::Int32 | Ty::UInt
                  => Some(format!("print(intToText({}))", expr)),
        Ty::Float => Some(format!("print(floatToText({}))", expr)),
        // Float32 is a distinct type from Float (does not unify — see
        // crates/typeck/src/unify.rs), so it needs its own conversion here,
        // not floatToText (which is typed exactly Ty::Float -> Ty::Text and
        // would fail to type-check against a Float32-typed expr).
        Ty::Float32 => Some(format!("print(float32ToText({}))", expr)),
        Ty::Bool  => Some(format!("print(boolToText({}))", expr)),
        Ty::Char  => Some(format!("print(Char.toText({}))", expr)),
        _ => None,
    }
}

/// Infer the type of `expr` by annotating a probe val as `Bool` and reading
/// the mismatch error.  Returns `Some(Bool)` if the expression really is Bool,
/// `Some(T)` for any other type, or `None` if parsing / inference fails.
fn infer_type(expr: &str, session: &[String]) -> Option<Ty> {
    let probe = format!("val _probe: Bool = ({})", expr);
    let combined = format!("{}\n{}\n", make_module_src(session, None), probe);
    let module = certo_parser::parse(&combined).ok()?;
    let mut env     = TypeEnv::new();
    let mut counter = 0u32;
    env.seed_builtins(&mut counter);
    certo_stdlib::seed_stdlib(&mut env, &mut counter);
    match certo_typeck::check_module_seeded(&module, env, counter) {
        Ok(()) => Some(Ty::Bool),
        // Certo reports: expected=<actual expr type>, found=<annotation type>.
        // So the real type is in `expected`.
        Err(errs) => errs.into_iter().find_map(|e| {
            if let TypeErrorKind::Mismatch { expected, .. } = e.kind {
                Some(expected)
            } else {
                None
            }
        }),
    }
}

// ------------------------------------------------------------------ //
// :type command
// ------------------------------------------------------------------ //

fn repl_show_type(expr: &str, session: &[String]) {
    match infer_type(expr, session) {
        Some(ty) => {
            let names = assign_var_names(&[&ty]);
            println!("-- : {}", ty.display_named(&names));
        }
        None => {
            // Can't infer — try as a statement to get a parse/type error.
            let probe = format!("val _probe: Bool = ({})", expr);
            let combined = format!("{}\n{}\n", make_module_src(session, None), probe);
            match certo_parser::parse(&combined) {
                Err(errs) => eprintln!("parse error: {}", errs.first().map(|e| e.to_string()).unwrap_or_default()),
                Ok(_) => eprintln!("could not infer type for: {}", expr),
            }
        }
    }
}

// ------------------------------------------------------------------ //
// Compile + run pipeline
// ------------------------------------------------------------------ //

enum EvalResult {
    Ok,
    TypeError(Vec<TypeError>),
    ParseError(String),
    CompileError,
    RuntimeError(i32),
}

fn try_compile_and_run(src: &str, _colour: bool) -> EvalResult {
    // Parse
    let module = match certo_parser::parse(src) {
        Ok(m) => m,
        Err(errs) => {
            let msg = errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ");
            return EvalResult::ParseError(msg);
        }
    };

    // Type-check
    let mut env     = TypeEnv::new();
    let mut counter = 0u32;
    env.seed_builtins(&mut counter);
    certo_stdlib::seed_stdlib(&mut env, &mut counter);
    if let Err(errs) = certo_typeck::check_module_seeded(&module, env, counter) {
        return EvalResult::TypeError(errs);
    }

    // Codegen
    let preamble = crate::REPL_PREAMBLE;
    let runtime  = certo_codegen::RUNTIME_HEADER;
    let stdlib_c = certo_stdlib::full_c_runtime_with_db(false);
    let module_c = certo_codegen::emit_module(
        &module,
        &certo_codegen::CodegenOptions { inline_runtime: false, export_public: false },
    );
    let module_c = module_c.lines()
        .filter(|l| {
            !l.contains("certo_runtime.h") &&
            !l.contains("#include <stdint.h>") &&
            !l.contains("#include <stdbool.h>") &&
            !l.contains("#include <stddef.h>")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let full_c = format!("{}{}\n{}\n{}", preamble, runtime, stdlib_c, module_c);

    // Write temp C file
    let tmp_c = match tempfile::Builder::new().prefix("certo_repl_").suffix(".c").tempfile() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {}", e);
            return EvalResult::CompileError;
        }
    };
    if let Err(e) = std::fs::write(tmp_c.path(), &full_c) {
        eprintln!("error: {}", e);
        return EvalResult::CompileError;
    }

    // Find C compiler
    let cc = match crate::find_cc() {
        Some(c) => c,
        None => {
            eprintln!("error: no C compiler found (tried clang, gcc, cc)");
            return EvalResult::CompileError;
        }
    };

    // Output binary path
    let tmp_dir = std::env::temp_dir();
    let bin_name = if cfg!(windows) { "certo_repl.exe" } else { "certo_repl" };
    let bin_path = tmp_dir.join(bin_name);

    let mut cmd = std::process::Command::new(&cc);
    cmd.arg(tmp_c.path())
       .arg("-o").arg(&bin_path)
       .arg("-O0")
       .arg("-Wno-int-to-pointer-cast")
       .arg("-Wno-pointer-to-int-cast")
       .arg("-Wno-int-conversion")
       .arg("-Wno-implicit-function-declaration")
       .arg("-Wno-deprecated-declarations");
    if cfg!(windows) {
        cmd.arg("-Xlinker").arg("/subsystem:console");
    } else {
        cmd.arg("-lm");
    }

    let status = match cmd.status() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error invoking {}: {}", cc, e);
            return EvalResult::CompileError;
        }
    };
    if !status.success() {
        return EvalResult::CompileError;
    }

    // Run the compiled binary, inheriting stdio
    let run_status = std::process::Command::new(&bin_path)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    // Ensure the next prompt starts on a fresh line even if the program
    // didn't end its output with a newline.
    println!();

    match run_status {
        Ok(s) if s.success() => EvalResult::Ok,
        Ok(s) => EvalResult::RuntimeError(s.code().unwrap_or(-1)),
        Err(e) => {
            eprintln!("error running REPL binary: {}", e);
            EvalResult::CompileError
        }
    }
}

// ------------------------------------------------------------------ //
// Source synthesis helpers
// ------------------------------------------------------------------ //

/// Build a complete module source string.
/// If `main_body` is Some, appends a main function containing the body.
///
/// When the body contains `?`, the body is placed inside a helper that returns
/// `Result<Unit, Text>` so the `?` operator can early-return on Err. The
/// helper's result is matched in main() and any Err is printed to stderr.
fn make_module_src(session: &[String], main_body: Option<&str>) -> String {
    let mut out = String::from("module Repl\n\nimport Stdlib.Core\n\n");
    for decl in session {
        out.push_str(decl);
        out.push_str("\n\n");
    }
    if let Some(body) = main_body {
        if body_uses_try(body) {
            // Wrap in a Result-returning helper so ? can early-return on Err.
            out.push_str("fn _repl_try(): Result<Unit, Text> [io] = {\n");
            for line in body.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("    Ok(())\n");
            out.push_str("}\n");
            out.push_str("fn main(): Unit [io] = {\n");
            out.push_str("    match _repl_try() {\n");
            out.push_str("        Ok(_)  => ()\n");
            out.push_str("        Err(e) => println(\"Error: \" ++ e)\n");
            out.push_str("    }\n");
            out.push_str("}\n");
        } else {
            out.push_str("fn main(): Unit [io] = {\n");
            for line in body.lines() {
                out.push_str("    ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("}\n");
        }
    }
    out
}

/// True when the body text contains a bare `?` operator — the Result
/// propagation operator — as opposed to `??` (null-coalesce).
fn body_uses_try(body: &str) -> bool {
    let chars: Vec<char> = body.chars().collect();
    let mut in_str = false;
    let mut in_line_comment = false;
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\n' => { in_line_comment = false; }
            '/' if !in_str && i + 1 < chars.len() && chars[i + 1] == '/' => {
                in_line_comment = true;
            }
            '"' if !in_line_comment => { in_str = !in_str; }
            '?' if !in_str && !in_line_comment => {
                let next = chars.get(i + 1).copied();
                let prev = if i > 0 { chars.get(i - 1).copied() } else { None };
                if next != Some('?') && prev != Some('?') {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Return true when the input has more `{`/`(` than `}`/`)`, meaning
/// the user is in the middle of a multi-line expression.
fn needs_continuation(s: &str) -> bool {
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    let mut in_str      = false;
    let mut in_char     = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' if !in_char => {
                in_str = !in_str;
            }
            '\'' if !in_str => {
                in_char = !in_char;
            }
            '\\' if in_str || in_char => {
                i += 1; // skip escaped char
            }
            '{' if !in_str && !in_char => depth_brace += 1,
            '}' if !in_str && !in_char => depth_brace -= 1,
            '(' if !in_str && !in_char => depth_paren += 1,
            ')' if !in_str && !in_char => depth_paren -= 1,
            _ => {}
        }
        i += 1;
    }
    depth_brace > 0 || depth_paren > 0
}
