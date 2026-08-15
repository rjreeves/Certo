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

pub fn cmd_repl(args: &[String]) {
    let mut dsn: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" => {
                i += 1;
                dsn = Some(
                    args.get(i).unwrap_or_else(|| crate::die("--connect requires a connection string", 2)).clone()
                );
            }
            "--help" | "-h" => {
                println!("Usage: certo repl [--connect <dsn>]");
                println!();
                println!("Start an interactive REPL.");
                println!("  --connect <dsn>  Make `conn(): Int` available — a fresh PostgreSQL connection per call");
                return;
            }
            other => {
                eprintln!("Unknown option: {}", other);
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let dsn = dsn.as_deref();

    println!("Certo v{} (c) SyntrA 2026", CERTO_VERSION);
    println!("Type :help for commands, :quit or . to exit.");
    if dsn.is_some() {
        println!("Connected — call `conn()` for a live database connection, e.g. `dbQuery(conn(), \"select 1\", [])`.");
    }
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
            if handle_command(raw, &mut session, dsn) {
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

        repl_eval(&input, &mut session, colour, dsn);
    }

    println!();
    println!("Bye.");
}

// ------------------------------------------------------------------ //
// Meta-commands
// ------------------------------------------------------------------ //

/// Returns true if the line was a REPL command (consumed).
fn handle_command(line: &str, session: &mut Vec<String>, dsn: Option<&str>) -> bool {
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
            repl_show_type(expr, session, dsn);
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

fn repl_eval(input: &str, session: &mut Vec<String>, colour: bool, dsn: Option<&str>) {
    if looks_like_decl(input) {
        eval_decl(input, session, colour, dsn);
    } else {
        eval_expr_or_stmt(input, session, colour, dsn);
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

fn eval_decl(input: &str, session: &mut Vec<String>, colour: bool, dsn: Option<&str>) {
    // Parse the declaration in the context of the full session.
    let test_src = make_module_src(session, None, dsn);
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

fn eval_expr_or_stmt(input: &str, session: &mut Vec<String>, colour: bool, dsn: Option<&str>) {
    // Probe the type of the input as an expression first.
    // If it's a print-able primitive, auto-wrap it so the value is displayed.
    // Skip the probe when the input uses `?` — the probe context is Unit-returning
    // and would produce a spurious type error before we get a chance to run it.
    if !body_uses_try(input) {
        if let Some(ty) = infer_type(input, session, dsn) {
            if let Some(print_stmt) = auto_print_for(&ty, input) {
                let src2 = make_module_src(session, Some(&print_stmt), dsn);
                match try_compile_and_run(&src2, colour, dsn) {
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
    let src = make_module_src(session, Some(input), dsn);
    match try_compile_and_run(&src, colour, dsn) {
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
fn infer_type(expr: &str, session: &[String], dsn: Option<&str>) -> Option<Ty> {
    let probe = format!("val _probe: Bool = ({})", expr);
    let combined = format!("{}\n{}\n", make_module_src(session, None, dsn), probe);
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

fn repl_show_type(expr: &str, session: &[String], dsn: Option<&str>) {
    match infer_type(expr, session, dsn) {
        Some(ty) => {
            let names = assign_var_names(&[&ty]);
            println!("-- : {}", ty.display_named(&names));
        }
        None => {
            // Can't infer — try as a statement to get a parse/type error.
            let probe = format!("val _probe: Bool = ({})", expr);
            let combined = format!("{}\n{}\n", make_module_src(session, None, dsn), probe);
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

fn try_compile_and_run(src: &str, _colour: bool, dsn: Option<&str>) -> EvalResult {
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
    // BACKLOG item 154: previously hardcoded `false` — DB support was never
    // linked into any REPL turn, so `dbConnect`/`dbQuery` etc. would fail to
    // link (`undefined symbol`) even if a DSN were somehow available. Only
    // pull in the DB C runtime (and, below, libpq itself) when `--connect`
    // was actually passed, so an ordinary non-DB REPL session — the common
    // case — doesn't pay extra compile time on every single turn.
    let stdlib_c = certo_stdlib::full_c_runtime_with_db(dsn.is_some());
    let module_c = certo_codegen::emit_module(
        &module,
        &certo_codegen::CodegenOptions { inline_runtime: false, export_public: false, line_directives: None },
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
    if dsn.is_some() {
        let (pg_inc, pg_lib) = crate::resolve_pg_paths();
        if let Some(inc) = &pg_inc {
            cmd.arg(format!("-I{}", inc));
        }
        if cfg!(windows) {
            if let Some(lib_dir) = &pg_lib {
                cmd.arg(format!("{}/libpq.lib", lib_dir));
            } else {
                cmd.arg("libpq.lib");
            }
        } else {
            if let Some(lib) = &pg_lib {
                cmd.arg(format!("-L{}", lib));
            }
            cmd.arg("-lpq");
        }
    }
    if cfg!(windows) {
        // `certo_codegen::emit_module` unconditionally generates a `wmain`
        // entry point on Windows (real Unicode argv via `CommandLineToArgvW`,
        // `crates/codegen/src/emit_module.rs`) for every compiled module —
        // this was already true before this item and applies regardless of
        // `--connect`. `certo build`'s own compile invocation already links
        // `-lshell32`/`-luser32` and sets the matching entry point
        // (`crates/cli/src/main.rs`); the REPL's own separate compile command
        // never did, so *any* REPL turn that reached this far failed to link
        // with `undefined symbol: CommandLineToArgvW` — confirmed directly
        // with a plain `println("hello")`, no DB/--connect involved at all.
        cmd.arg("-Xlinker").arg("/subsystem:console");
        cmd.arg("-Xlinker").arg("/entry:wmainCRTStartup");
        cmd.arg("-luser32");
        cmd.arg("-lshell32");
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
fn make_module_src(session: &[String], main_body: Option<&str>, dsn: Option<&str>) -> String {
    let mut out = String::from("module Repl\n\nimport Stdlib.Core\n");
    // BACKLOG item 154: `conn()` — a zero-arg function, not a `val` — gives
    // a fresh connection on every call. There's no persistent interpreter
    // state to hold a live connection across turns anyway (each turn
    // recompiles the whole session and runs a brand-new subprocess), so
    // "connected" already means "reconnect transparently every time," not
    // "one long-lived socket" — a function call is actually the more
    // honest shape for that, not just a workaround.
    //
    // Deliberately a top-level `fn`, not a top-level `val conn = dbConnect(...)`
    // — found via direct testing that a top-level `val` whose initializer is
    // a function call is a real, separate, pre-existing codegen bug:
    // `emit_module.rs` emits a bare `static void* certo_conn = /* expr */0;`
    // for it, silently discarding the real initializer, so the connection
    // handle was always NULL and every query failed with "connection failed"
    // — confirmed by inspecting the generated C directly, not guessed. A
    // top-level *function* isn't affected (its body is a real function,
    // not a static initializer) and — unlike a function-local `val`, which
    // was the first fix attempted here — is visible from *both* a bare
    // expression typed at the prompt (wrapped into `main()`'s body below)
    // *and* a session-level `val`/`fn` the user defines (a top-level
    // declaration, module-scoped like any other), so `dbQuery(conn(), ...)`
    // works identically in both positions.
    if dsn.is_some() {
        out.push_str("import Stdlib.Db\n\n");
    } else {
        out.push('\n');
    }
    if let Some(dsn) = dsn {
        out.push_str(&format!("fn conn(): Int [io] = dbConnect(\"{}\")\n\n", escape_certo_string(dsn)));
    }
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

/// Escape a raw DSN string for embedding as a Certo `"..."` literal.
fn escape_certo_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------ //
    // BACKLOG item 154: `certo repl --connect <dsn>`
    // ------------------------------------------------------------------ //

    #[test]
    fn escape_handles_backslash_and_quote() {
        assert_eq!(escape_certo_string(r#"host=localhost"#), "host=localhost");
        assert_eq!(escape_certo_string(r#"pass"word"#), r#"pass\"word"#);
        assert_eq!(escape_certo_string(r"C:\path"), r"C:\\path");
    }

    /// Find the string-literal argument of the top-level `fn conn(): Int
    /// [io] = dbConnect("...")` (see `make_module_src`'s own doc comment
    /// for why it's a function, not a `val`).
    fn find_conn_dsn_literal(module: &certo_ast::module::Module) -> Option<String> {
        for d in &module.decls {
            let certo_ast::decl::Decl::Fn(f) = &d.node else { continue };
            if f.name.node != "conn" { continue; }
            let body = f.body.as_ref()?;
            let certo_ast::expr::Expr::App { args, .. } = &body.node else { continue };
            if let certo_ast::expr::Expr::Lit { value: certo_ast::expr::Lit::String(s), .. } = &args[0].value.node {
                return Some(s.clone());
            }
        }
        None
    }

    #[test]
    fn escaped_dsn_round_trips_through_the_real_lexer_and_parser() {
        // The escaped literal must actually parse back to the exact
        // original DSN — not just "look escaped" — since a malformed
        // escape would either fail to parse or silently truncate/corrupt
        // the connection string embedded in the synthesized module source.
        let dsn = r#"host=localhost dbname="my db" password=a\b"#;
        let src = make_module_src(&[], Some("println(\"x\")"), Some(dsn));
        let module = certo_parser::parse(&src).expect("synthesized module must parse");
        assert_eq!(find_conn_dsn_literal(&module).as_deref(), Some(dsn));
    }

    #[test]
    fn no_dsn_means_no_conn_function_or_db_import() {
        let src = make_module_src(&[], Some("println(\"x\")"), None);
        assert!(!src.contains("import Stdlib.Db"));
        assert!(!src.contains("fn conn"));
    }

    #[test]
    fn dsn_present_injects_conn_function_and_db_import() {
        let src = make_module_src(&[], Some("println(\"x\")"), Some("host=localhost"));
        assert!(src.contains("import Stdlib.Db"));
        assert!(src.contains("fn conn(): Int [io] = dbConnect(\"host=localhost\")"));
        // `conn()` must be declared before session declarations and before
        // main() so both a session-level val/fn and a bare per-turn
        // expression can reference it — a top-level *function*, unlike the
        // function-local `val` first attempted here, is visible from both
        // positions (see make_module_src's own doc comment for why a
        // top-level `val` isn't safe to use here at all).
        let conn_pos = src.find("fn conn").expect("conn() present");
        let main_pos = src.find("fn main").expect("main() present");
        assert!(conn_pos < main_pos, "conn() must be declared before main()");
    }

    #[test]
    fn no_main_body_still_injects_conn_function_when_dsn_present() {
        // Unlike the earlier function-local design, `conn()` is a
        // top-level declaration — it doesn't depend on there being a
        // main() body at all, so the declaration-probe/type-probe modules
        // (`main_body: None`) still get it (needed so a session-level
        // `fn foo() = { dbQuery(conn(), ...) }` type-checks).
        let src = make_module_src(&[], None, Some("host=localhost"));
        assert!(src.contains("fn conn(): Int [io] = dbConnect(\"host=localhost\")"));
    }

    #[test]
    fn conn_function_precedes_the_try_wrapper_for_question_mark_bodies() {
        let src = make_module_src(&[], Some("dbExec(conn(), \"x\", [])?"), Some("host=localhost"));
        assert!(src.contains("fn _repl_try"));
        let conn_pos = src.find("fn conn").expect("conn() present");
        let try_pos = src.find("fn _repl_try").expect("_repl_try present");
        assert!(conn_pos < try_pos, "conn() must be declared before _repl_try()");
    }
}
