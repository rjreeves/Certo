//! Build a C translation unit from a Certo module that contains test declarations.
//!
//! Strategy:
//!   1. Clone the module, inject a synthetic `fn __test_<N>()` for every
//!      `test`, `dbTest`, and `property` declaration.
//!   2. Emit the augmented module to C via `certo_codegen::emit_module`.
//!   3. Append a `main()` that accepts a test name as `argv[1]` and calls
//!      the matching test function (exit 0 on pass, 1 on panic/assertion).
//!
//! The runner (see `run.rs`) spawns one subprocess per test so failures are
//! isolated and the process-exit code is the definitive pass/fail signal.

use certo_ast::{
    decl::{Decl, FnDecl, TestExpectation, ValidatorDecl},
    expr::{Arg, Expr, ExpectMatcher, MatchArm, Stmt},
    module::Module,
    pattern::Pattern,
    span::{Span, S},
    types::ModulePath,
};
use certo_codegen::{emit_module, CodegenOptions, c_fn_name, c_ident};

use crate::gen::{min_argv_width, param_gen_types, GenType, TypeDecls};

/// Information about a single synthesized test entry point.
#[derive(Debug, Clone)]
pub struct TestEntry {
    /// Human-readable name from the source (`test "name" { ... }`).
    pub display_name: String,
    /// Kind of test block.
    pub kind: TestKind,
    /// C function name emitted for this test.
    pub c_fn_name: String,
    /// `property` parameters the runner generates values for (name, type).
    /// Empty for `test`/`dbTest` and for a `property` with no declared
    /// inputs — both run exactly once, unchanged from before this existed.
    pub params: Vec<(String, GenType)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestKind {
    Unit,
    Db,
    Property,
}

impl TestKind {
    pub fn label(&self) -> &'static str {
        match self {
            TestKind::Unit     => "test",
            TestKind::Db       => "dbTest",
            TestKind::Property => "property",
        }
    }
}

/// Whether the compiled test binary needs the PostgreSQL C runtime
/// (`DB_C`/`DBQUERY_C`/`DBMUTATION_C`) linked in — mirrors `certo build`'s
/// own `uses_db` import-based detection (`crates/cli/src/main.rs`), plus
/// unconditionally true whenever any `dbTest` is present: a `dbTest`'s own
/// synthesized wrapper (`build_db_test_fn`, below) always calls
/// `dbConnect`/`dbBegin`/`dbRollback` regardless of whether the source file
/// bothered to write `import Stdlib.Db` itself. Exposed (not private) so
/// `lib.rs` can pass the same answer to the linker — `certo test` links a
/// separate binary per invocation from `compile.rs`, entirely apart from
/// `build_harness`'s own C-runtime-selection use of this same check.
pub fn uses_db(module: &Module, entries: &[TestEntry]) -> bool {
    if entries.iter().any(|e| e.kind == TestKind::Db) {
        return true;
    }
    module.imports.iter().any(|imp| {
        let segs: Vec<&str> = imp.path.segments.iter().map(|s| s.node.as_str()).collect();
        segs == ["Stdlib", "Db"] || segs == ["Db"]
            || segs == ["Stdlib", "DbQuery"] || segs == ["DbQuery"]
            || segs == ["Stdlib", "DbMutation"] || segs == ["DbMutation"]
    })
}

/// Sanitise a test display name into a valid C identifier fragment.
pub fn sanitise_name(name: &str) -> String {
    let s: String = name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    // Collapse repeated underscores and strip leading digits.
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "test".into() } else { s }
}

/// Build a C source string that exercises all test declarations in `module`.
///
/// Returns `(c_source, entries)` where `entries` describe each test so the
/// caller can spawn sub-processes. Fails if a `property` block declares a
/// parameter whose type has no value generator yet (see `gen::GenType`).
///
/// `coverage_source`, when `Some((filename, source))`, opts codegen into
/// emitting `#line` directives mapping the generated C back to the original
/// `.cto` file (BACKLOG item 126) — passed straight through to
/// `CodegenOptions::line_directives`; `None` for every normal `certo test`
/// run (unaffected, identical output to before this parameter existed).
pub fn build_harness(module: &Module, coverage_source: Option<(String, String)>) -> Result<(String, Vec<TestEntry>), crate::error::TestRunnerError> {
    let mut augmented = module.clone();
    // BACKLOG item 222 — `certo test` previously never expanded `validator`/
    // `statemachine` declarations at all: a test file declaring a
    // `validator` and calling `V.validate(...)` from an ordinary `test`
    // block already failed to compile under `certo test`, independent of
    // `ruleTest`/`validatorTest` support below, which fundamentally needs
    // this too (a `ruleTest`/`validatorTest`'s own synthesized call target
    // — `V.validate`/`V.someRule` — doesn't exist as a real function until
    // this runs). Reuses the exact same expansion `certo build`/`check`/
    // `run` already call (`crates/cli/src/main.rs`), now shared via
    // `certo_codegen::expand` instead of being CLI-only.
    let mut combined_src = String::new();
    certo_codegen::expand_state_machines(&mut augmented, &mut combined_src)
        .map_err(|(wrapped, _)| crate::error::TestRunnerError::ValidatorExpansionFailed(wrapped))?;
    certo_codegen::expand_validators(&mut augmented, &mut combined_src)
        .map_err(|(wrapped, _)| crate::error::TestRunnerError::ValidatorExpansionFailed(wrapped))?;
    let mut entries: Vec<TestEntry> = Vec::new();
    let zero_span = Span { start: 0, end: 0 };
    // Record/sum-type shapes for property parameter generation are read
    // directly off this module's own `type X = ...` declarations — not a
    // typeck pass, just the same AST the property itself was declared in
    // (BACKLOG item 124).
    let type_decls = TypeDecls::from_module(module);

    // Synthesise a wrapper FnDecl for every test/dbTest/property block.
    for sdecl in &module.decls {
        let (display_name, params, body, kind) = match &sdecl.node {
            Decl::Test(t)     => (t.name.clone(), vec![], t.body.clone(), TestKind::Unit),
            Decl::DbTest(t)   => (t.name.clone(), vec![], t.body.clone(), TestKind::Db),
            Decl::Property(t) => (t.name.clone(), t.params.clone(), t.body.clone(), TestKind::Property),
            _                 => continue,
        };

        let gen_params = param_gen_types(&params, &type_decls).map_err(|param_name| {
            crate::error::TestRunnerError::UnsupportedPropertyParamType {
                property_name: display_name.clone(),
                param_name,
            }
        })?;

        let safe  = sanitise_name(&display_name);
        let idx   = entries.len();
        let fn_id = format!("__test_{}_{}", idx, safe);

        let fn_decl = if kind == TestKind::Db {
            build_db_test_fn(&fn_id, &body)?
        } else {
            FnDecl {
                is_async:    false,
                is_pub:      false,
                name:        S::new(fn_id.clone(), zero_span),
                type_params: vec![],
                params,
                ret_ty:      None,
                effects:     None,
                body:        Some(body),
                is_extern:   false,
                export_name: None,
                span:        zero_span,
            }
        };
        augmented.decls.push(S::new(Decl::Fn(fn_decl), zero_span));

        entries.push(TestEntry {
            display_name,
            kind,
            c_fn_name: fn_id.clone(),
            params: gen_params,
        });
    }

    // BACKLOG item 222 — `ruleTest`/`validatorTest`. Looked up against the
    // *original* `module` (validators are never removed by expansion above,
    // only supplemented with the real functions this synthesizes calls to —
    // see `certo_codegen::expand::expand_validators`'s own doc comment).
    for sdecl in &module.decls {
        let (label, expect, call, span) = match &sdecl.node {
            Decl::RuleTest(rt) => {
                let (validator_name, rule_name) = match rt.validator.as_slice() {
                    [v, r] => (v.node.as_str(), r.node.as_str()),
                    _ => return Err(crate::error::TestRunnerError::UnknownValidatorRef {
                        label: rt.label.clone(),
                        path: rt.validator.iter().map(|i| i.node.as_str()).collect::<Vec<_>>().join("."),
                    }),
                };
                let v = find_validator(module, validator_name).ok_or_else(|| crate::error::TestRunnerError::UnknownValidatorRef {
                    label: rt.label.clone(),
                    path: format!("{}.{}", validator_name, rule_name),
                })?;
                if !v.rules.iter().any(|r| r.name.node == rule_name) {
                    return Err(crate::error::TestRunnerError::UnknownValidatorRef {
                        label: rt.label.clone(),
                        path: format!("{}.{}", validator_name, rule_name),
                    });
                }
                let call = build_validator_call(validator_name, rule_name, &rt.entity, &rt.context, !v.context.is_empty(), rt.span);
                (rt.label.clone(), &rt.expect, call, rt.span)
            }
            Decl::ValidatorTest(vt) => {
                let v = find_validator(module, &vt.validator.node).ok_or_else(|| crate::error::TestRunnerError::UnknownValidatorRef {
                    label: vt.label.clone(),
                    path: vt.validator.node.clone(),
                })?;
                let call = build_validator_call(&vt.validator.node, "validate", &vt.entity, &vt.context, !v.context.is_empty(), vt.span);
                (vt.label.clone(), &vt.expect, call, vt.span)
            }
            _ => continue,
        };
        let body = build_expectation_body(call, expect, span);

        let safe  = sanitise_name(&label);
        let idx   = entries.len();
        let fn_id = format!("__test_{}_{}", idx, safe);
        let fn_decl = FnDecl {
            is_async:    false,
            is_pub:      false,
            name:        S::new(fn_id.clone(), zero_span),
            type_params: vec![],
            params:      vec![],
            ret_ty:      None,
            effects:     None,
            body:        Some(body),
            is_extern:   false,
            export_name: None,
            span:        zero_span,
        };
        augmented.decls.push(S::new(Decl::Fn(fn_decl), zero_span));
        entries.push(TestEntry {
            display_name: label,
            kind: TestKind::Unit,
            c_fn_name: fn_id,
            params: vec![],
        });
    }

    // Emit the augmented module (all user functions + test wrappers).
    // Order must match the CLI: system includes → RUNTIME_HEADER → stdlib C → user code.
    // Same preamble as `certo build` (crates/cli/src/main.rs) — Windows needs
    // WIN32_LEAN_AND_MEAN + winsock2.h before windows.h (RUNTIME_HEADER pulls
    // in windows.h for its concurrency primitives) to avoid a WinSock1-vs-2
    // header conflict, and _USE_MATH_DEFINES to expose M_PI/M_E from
    // <math.h> on MSVC/clang-cl — without it, any test file (property tests
    // included) that pulls in Stdlib.Math failed to compile at all.
    let opts = CodegenOptions { inline_runtime: false, export_public: false, line_directives: coverage_source };
    let preamble = "#ifdef _WIN32\n\
                    #  ifndef WIN32_LEAN_AND_MEAN\n\
                    #    define WIN32_LEAN_AND_MEAN\n\
                    #  endif\n\
                    #  ifndef _USE_MATH_DEFINES\n\
                    #    define _USE_MATH_DEFINES\n\
                    #  endif\n\
                    #  include <winsock2.h>\n\
                    #  include <ws2tcpip.h>\n\
                    #  pragma comment(lib, \"ws2_32.lib\")\n\
                    #endif\n\
                    #include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\
                    #include <inttypes.h>\n#include <stdarg.h>\n\
                    #define _CRT_SECURE_NO_WARNINGS\n\n";
    let mut c_src = preamble.to_string();
    c_src.push_str(certo_codegen::RUNTIME_HEADER);
    c_src.push('\n');
    c_src.push_str(&certo_stdlib::full_c_runtime_with_db(uses_db(module, &entries)));
    c_src.push('\n');
    // Strip the `#include "certo_runtime.h"` stub and duplicate system includes.
    let module_out = emit_module(&augmented, &opts);
    for line in module_out.lines() {
        if !line.contains("certo_runtime.h") &&
           !line.contains("#include <stdint.h>") &&
           !line.contains("#include <stdbool.h>") &&
           !line.contains("#include <stddef.h>") {
            c_src.push_str(line);
            c_src.push('\n');
        }
    }

    // Append a dispatching main().
    c_src.push_str("\n#include <stdio.h>\n#include <string.h>\n#include <stdlib.h>\n\n");
    c_src.push_str("int main(int argc, char** argv) {\n");
    c_src.push_str("    if (argc < 2) {\n");
    // Print all test names (used by the runner to discover tests).
    c_src.push_str("        /* list mode: print all test names, one per line */\n");
    for (i, e) in entries.iter().enumerate() {
        c_src.push_str(&format!(
            "        printf(\"%s\\n\", \"{}\");\n",
            e.display_name.replace('"', "\\\"")
        ));
        let _ = i;
    }
    c_src.push_str("        return 0;\n");
    c_src.push_str("    }\n\n");

    c_src.push_str("    const char* target = argv[1];\n");
    for e in &entries {
        let escaped = e.display_name.replace('"', "\\\"");
        let mangled = c_fn_name(&e.c_fn_name);
        if e.params.is_empty() {
            // Forward-declare the test function (int64_t because Ty::Error emits int64_t).
            c_src.push_str(&format!("    int64_t {mangled}(void);\n"));
            c_src.push_str(&format!(
                "    if (strcmp(target, \"{escaped}\") == 0) {{ {mangled}(); return 0; }}\n",
            ));
        } else {
            // Property test: argv[2..] carries the generated values, encoded
            // by the runner's driver (`gen::GenValue::to_args`) and decoded
            // here via a runtime cursor (`argv_i`) rather than fixed indices
            // — a `List<T>` parameter's encoded width isn't known until its
            // length prefix is read at runtime (BACKLOG item 124). The
            // `argc` check below is a cheap minimum-width sanity net, not an
            // exact bound (exact width is runtime-dependent for anything
            // containing a `List`) — a mismatch is a driver/harness bug, not
            // a user-facing scenario.
            let c_param_tys: Vec<String> = e.params.iter().map(|(_, gt)| gen_type_c_str(gt)).collect();
            c_src.push_str(&format!("    int64_t {mangled}({});\n", c_param_tys.join(", ")));
            c_src.push_str(&format!(
                "    if (strcmp(target, \"{escaped}\") == 0) {{\n",
            ));
            let min_width: usize = e.params.iter().map(|(_, gt)| min_argv_width(gt)).sum();
            c_src.push_str(&format!(
                "        if (argc < {}) {{ fprintf(stderr, \"{escaped}: expected at least {} generated argument(s)\\n\"); return 2; }}\n",
                2 + min_width, min_width,
            ));
            c_src.push_str("        int argv_i = 2;\n");
            let mut stmts = String::new();
            let mut tmp = 0u32;
            let call_args: Vec<String> = e.params.iter()
                .map(|(_, gt)| emit_decode(gt, &mut stmts, &mut tmp))
                .collect();
            c_src.push_str(&stmts);
            c_src.push_str(&format!("        {mangled}({});\n", call_args.join(", ")));
            c_src.push_str("        return 0;\n");
            c_src.push_str("    }\n");
        }
    }

    c_src.push_str("    fprintf(stderr, \"unknown test: %s\\n\", target);\n");
    c_src.push_str("    return 1;\n");
    c_src.push_str("}\n");

    Ok((c_src, entries))
}

/// Build the synthesized wrapper function for a `dbTest` block, with a real
/// auto-rollback transaction around the user's own body — BACKLOG item 165.
/// Previously `dbTest` was a cosmetic label only: its body ran exactly like
/// a plain `test` block, with zero transaction wrapping anywhere in this
/// crate, so any DB write it performed was permanent — directly
/// contradicting the spec's own "auto-rollback after test" claim.
///
/// Generates real Certo *source text* and re-parses it with the real
/// parser, rather than hand-constructing AST nodes for the wrapper — the
/// same approach BACKLOG item 87 (`@ui.generate`) already established for
/// synthesizing new Certo code from a compiler pass, which guarantees the
/// output is exactly as valid as anything a user could type. The user's
/// own body is re-printed via `certo_fmt::fmt_expr` (not the original
/// source slice — `build_harness` only has the parsed AST) and spliced in
/// verbatim as a nested block statement.
///
/// The connection is opened from `DATABASE_URL` (env var, falling back to
/// `.env` — the exact resolution `getEnv` already performs) and bound to
/// `conn`, so the test body can reference `conn` directly for its own
/// `dbQuery`/`dbExec`/query-builder calls, all running inside the same
/// transaction that gets rolled back afterward. Rollback is only called
/// explicitly on the *success* path — if the body panics (e.g. a failed
/// `assert`), the whole test subprocess aborts immediately (this crate's
/// own one-subprocess-per-test isolation model, see this module's top-level
/// doc comment) without ever reaching the explicit `dbRollback` call; the
/// still-open connection is then torn down at process exit, which discards
/// any uncommitted transaction on the Postgres side just as reliably as an
/// explicit rollback would — `dbCommit` is never called on this connection
/// under any path, by design, since the whole point is these writes must
/// never persist.
fn build_db_test_fn(fn_id: &str, body: &S<Expr>) -> Result<FnDecl, crate::error::TestRunnerError> {
    let body_src = certo_fmt::fmt_expr(&body.node, 1);
    let wrapper_src = format!(
        "module __DbTestWrapper\n\
         fn {fn_id}() = {{\n    \
             val __dbtest_dsn = match getEnv(\"DATABASE_URL\") {{\n        \
                 Some(v) => v\n        \
                 None => panic(\"dbTest requires DATABASE_URL to be set (env var or .env file)\")\n    \
             }}\n    \
             val conn = dbConnect(__dbtest_dsn)\n    \
             dbBegin(conn)\n    \
             {body_src}\n    \
             dbRollback(conn)\n\
         }}\n"
    );
    let parsed = certo_parser::parse(&wrapper_src).map_err(|errs| {
        crate::error::TestRunnerError::ParseError(format!(
            "internal error: failed to parse synthesized dbTest wrapper for \"{fn_id}\": {:?}",
            errs
        ))
    })?;
    parsed.decls.into_iter()
        .find_map(|d| match d.node { Decl::Fn(f) => Some(f), _ => None })
        .ok_or_else(|| crate::error::TestRunnerError::ParseError(
            format!("internal error: synthesized dbTest wrapper for \"{fn_id}\" produced no fn decl")
        ))
}

// ------------------------------------------------------------------ //
// BACKLOG item 222 — `ruleTest`/`validatorTest` synthesis
// ------------------------------------------------------------------ //

fn find_validator<'a>(module: &'a Module, name: &str) -> Option<&'a ValidatorDecl> {
    module.decls.iter().find_map(|d| match &d.node {
        Decl::Validator(v) if v.name.node == name => Some(v),
        _ => None,
    })
}

fn path_expr(name: &str, span: Span) -> S<Expr> {
    S::new(Expr::Path {
        path: ModulePath { segments: vec![S::new(name.to_string(), span)], span },
        span,
    }, span)
}

/// Builds `{validator}.{method}(entity[, context])` — an ordinary dot-call,
/// the same shape `OrderSubmit.customer_active(...)`/`OrderSubmit.validate(...)`
/// already resolve through via `c_fn_name`'s existing dot/underscore
/// normalization (no new call-resolution mechanism needed). `entity`/
/// `context` are the test declaration's own already-parsed expressions,
/// reused verbatim — not re-parsed or re-formatted.
fn build_validator_call(validator: &str, method: &str, entity: &S<Expr>, context: &S<Expr>, has_context: bool, span: Span) -> S<Expr> {
    let func = S::new(Expr::Field {
        expr:  Box::new(path_expr(validator, span)),
        field: S::new(method.to_string(), span),
        span,
    }, span);
    let mut args = vec![Arg { label: None, value: entity.clone(), span }];
    if has_context {
        args.push(Arg { label: None, value: context.clone(), span });
    }
    S::new(Expr::App { func: Box::new(func), args, span }, span)
}

/// Builds the assertion body for one `ruleTest`/`validatorTest`, wrapping
/// `call` (the validator/rule invocation built above) with the *already-
/// existing* `expect(...)` matcher machinery (`ExpectMatcher`, used today by
/// ordinary `test` blocks) — `expect(x).toBe(y)` and `x.toBe(y)` are
/// equivalent (`expect` is a pure identity function, purely for
/// readability), so `call` is used directly as `ExpectAssertion.actual`
/// with no `expect(...)` wrapper needed.
fn build_expectation_body(call: S<Expr>, expect: &TestExpectation, span: Span) -> S<Expr> {
    match expect {
        TestExpectation::Pass => S::new(Expr::ExpectAssertion {
            actual: Box::new(call), matcher: ExpectMatcher::ToBeOk, span,
        }, span),
        TestExpectation::Fail { with: None } => S::new(Expr::ExpectAssertion {
            actual: Box::new(call), matcher: ExpectMatcher::ToBeErr, span,
        }, span),
        TestExpectation::Fail { with: Some(expected) } => {
            // val __result = <call>
            // __result.toBeErr()
            // match __result { Err(e) => e.toBe(expected), Ok(_) => () }
            let result_name = "__validator_test_result";
            let val_stmt = Stmt::Val {
                pattern: S::new(Pattern::Ident { name: S::new(result_name.to_string(), span), span }, span),
                ty: None,
                value: call,
                span,
            };
            let is_err_stmt = Stmt::Expr {
                expr: S::new(Expr::ExpectAssertion {
                    actual: Box::new(path_expr(result_name, span)),
                    matcher: ExpectMatcher::ToBeErr,
                    span,
                }, span),
                span,
            };
            let err_binding = "__validator_test_err";
            let err_arm = MatchArm {
                pattern: S::new(Pattern::Constructor {
                    path: ModulePath { segments: vec![S::new("Err".into(), span)], span },
                    fields: vec![S::new(Pattern::Ident { name: S::new(err_binding.to_string(), span), span }, span)],
                    span,
                }, span),
                guard: None,
                body: S::new(Expr::ExpectAssertion {
                    actual: Box::new(path_expr(err_binding, span)),
                    matcher: ExpectMatcher::ToBe(Box::new(expected.clone())),
                    span,
                }, span),
                span,
            };
            let ok_arm = MatchArm {
                pattern: S::new(Pattern::Constructor {
                    path: ModulePath { segments: vec![S::new("Ok".into(), span)], span },
                    fields: vec![S::new(Pattern::Wildcard { span }, span)],
                    span,
                }, span),
                guard: None,
                body: S::new(Expr::Lit { value: certo_ast::expr::Lit::Unit, span }, span),
                span,
            };
            let match_stmt = Stmt::Expr {
                expr: S::new(Expr::Match {
                    scrutinee: Box::new(path_expr(result_name, span)),
                    arms: vec![err_arm, ok_arm],
                    span,
                }, span),
                span,
            };
            S::new(Expr::Block { stmts: vec![val_stmt, is_err_stmt, match_stmt], span }, span)
        }
    }
}

/// The C parameter type for a decoded value of `gt` — must match exactly
/// what `certo_codegen::emit_module`'s own `ty_to_c`/`field_c_ty` produce
/// for the same declared type, since the property wrapper function's real
/// signature is compiled through that same, ordinary path.
fn gen_type_c_str(gt: &GenType) -> String {
    match gt {
        GenType::Int   => "int64_t".to_string(),
        GenType::Float => "double".to_string(),
        GenType::Bool  => "bool".to_string(),
        GenType::Text  => "certo_text_t".to_string(),
        GenType::List(_) => "void*".to_string(),
        GenType::Record { type_name, .. } => c_ident(type_name),
        GenType::Sum { type_name, .. } => c_ident(type_name),
    }
}

/// Box a decoded list element into the pointer-sized slot `certo_list_push`
/// expects — the exact same convention `crates/codegen/src/emit_mir.rs`'s
/// private `box_value` uses for every list/tuple element in the compiler
/// generally (`Float` is bit-cast via `__certo_f2i`; everything else here is
/// already pointer-sized). `gt` is never `Record`/`Sum` — `resolve_gen_type`
/// already refuses a struct-shaped list element (see `gen::GenType`'s doc
/// comment for why).
fn box_list_element(expr: &str, gt: &GenType) -> String {
    match gt {
        GenType::Float => format!("(void*)__certo_f2i({expr})"),
        _ => format!("(void*)(intptr_t)({expr})"),
    }
}

/// Emit C code decoding one value of `gt` from `argv`, advancing the
/// runtime cursor `argv_i` (declared once per dispatch branch, see
/// `build_harness`) by however many slots this value actually occupies.
/// Statements needed to materialize the decoded value are appended to
/// `out`; the return value is always a plain expression (a variable name,
/// for everything below) usable directly as a call argument. `tmp` is a
/// per-dispatch-branch unique-name counter, shared across this call's whole
/// recursion so nested temporaries (e.g. a `List<List<Int>>`, or a record
/// field that is itself a `List<T>`) never collide.
///
/// Every decode — including a plain scalar — materializes through its own
/// statement rather than an inline `argv[argv_i++]` expression, even though
/// a single one would be safe alone: as soon as *two* decoded values become
/// sibling operands of the same unsequenced C expression (a compound
/// literal's field initializers, or two arguments in one call), evaluation
/// order between them is unspecified and two `argv_i++` side effects in the
/// same unsequenced expression is undefined behavior — confirmed for real
/// via `-Wunsequenced` on a two-field record parameter while verifying this
/// item, not theorized. Giving each decode its own statement forces
/// left-to-right evaluation in exactly the order these functions already
/// emit them in (matching the encoder's own field/element order), so this
/// is correct for any number of sibling params/fields, not just one.
///
/// A `List` needs an actual loop (its length isn't known until `argv[argv_i]`
/// is read at runtime), so it materializes into a fresh `CertoList*` local.
/// A `Record` decodes to a compound literal (`(Point){ .x = ..., .y = ... }`),
/// matching exactly how `certo_codegen::emit_mir`'s own `AggregateKind::Record`
/// constructs one — each field is still decoded into its own statement
/// first, per the sequencing note above, so the literal itself only ever
/// references already-materialized variables. A `Sum` reads its variant
/// tag, then an `if`/`else if` chain — each branch decoding only *that*
/// variant's own fields, since a different variant can consume a different
/// number of argv slots — and calls the real constructor function/constant
/// the compiler already emitted for it (`certo_<snake_case_variant_name>`,
/// via `c_fn_name`, exactly matching `emit_module.rs`'s own naming for it).
fn emit_decode(gt: &GenType, out: &mut String, tmp: &mut u32) -> String {
    match gt {
        GenType::Int => {
            *tmp += 1;
            let v = format!("_v{}", *tmp);
            out.push_str(&format!("        int64_t {v} = (int64_t)atoll(argv[argv_i++]);\n"));
            v
        }
        GenType::Float => {
            *tmp += 1;
            let v = format!("_v{}", *tmp);
            out.push_str(&format!("        double {v} = atof(argv[argv_i++]);\n"));
            v
        }
        GenType::Bool => {
            *tmp += 1;
            let v = format!("_v{}", *tmp);
            out.push_str(&format!("        bool {v} = (strcmp(argv[argv_i++], \"true\") == 0);\n"));
            v
        }
        GenType::Text => {
            *tmp += 1;
            let v = format!("_v{}", *tmp);
            out.push_str(&format!("        certo_text_t {v} = argv[argv_i++];\n"));
            v
        }
        GenType::List(inner) => {
            *tmp += 1;
            let id = *tmp;
            let n   = format!("_n{id}");
            let lst = format!("_lst{id}");
            let k   = format!("_k{id}");
            out.push_str(&format!("        int64_t {n} = (int64_t)atoll(argv[argv_i++]);\n"));
            out.push_str(&format!("        CertoList* {lst} = certo_list_new();\n"));
            out.push_str(&format!("        for (int64_t {k} = 0; {k} < {n}; {k}++) {{\n"));
            let mut body = String::new();
            let elem_expr = emit_decode(inner, &mut body, tmp);
            let boxed = box_list_element(&elem_expr, inner);
            out.push_str(&body);
            out.push_str(&format!("            {lst} = certo_list_push({lst}, {boxed});\n"));
            out.push_str("        }\n");
            lst
        }
        GenType::Record { type_name, fields } => {
            let inits: Vec<String> = fields.iter().map(|(fname, fgt)| {
                let expr = emit_decode(fgt, out, tmp);
                format!(".{fname} = {expr}")
            }).collect();
            format!("(({}){{ {} }})", c_ident(type_name), inits.join(", "))
        }
        GenType::Sum { type_name, variants } => {
            *tmp += 1;
            let id = *tmp;
            let tag  = format!("_tag{id}");
            let dest = format!("_sum{id}");
            out.push_str(&format!("        int64_t {tag} = (int64_t)atoll(argv[argv_i++]);\n"));
            out.push_str(&format!("        {} {dest};\n", c_ident(type_name)));
            for (i, variant) in variants.iter().enumerate() {
                let kw = if i == 0 { "if" } else { "else if" };
                out.push_str(&format!("        {kw} ({tag} == {i}) {{\n"));
                let mut body = String::new();
                let arg_exprs: Vec<String> = variant.fields.iter()
                    .map(|fgt| emit_decode(fgt, &mut body, tmp))
                    .collect();
                out.push_str(&body);
                let cname = c_fn_name(&variant.name);
                let value_expr = if variant.fields.is_empty() {
                    cname
                } else {
                    format!("{}({})", cname, arg_exprs.join(", "))
                };
                out.push_str(&format!("            {dest} = {};\n", value_expr));
                out.push_str("        }\n");
            }
            out.push_str(&format!(
                "        else {{ fprintf(stderr, \"internal error: bad variant tag for {type_name}\\n\"); exit(2); }}\n",
            ));
            dest
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitise_basic() {
        assert_eq!(sanitise_name("adds two numbers"), "adds_two_numbers");
        assert_eq!(sanitise_name("foo-bar!"), "foo_bar");
        assert_eq!(sanitise_name(""), "test");
        assert_eq!(sanitise_name("123abc"), "123abc");
    }

    #[test]
    fn sanitise_no_leading_underscore() {
        // trim_matches strips outer underscores
        assert_eq!(sanitise_name("___"), "test");
    }

    #[test]
    fn build_harness_empty_module_no_entries() {
        use certo_ast::module::Module;
        use certo_ast::span::Span;

        let m = Module {
            path:    certo_ast::types::ModulePath {
                segments: vec![S::new("Test".into(), Span { start: 0, end: 0 })],
                span:     Span { start: 0, end: 0 },
            },
            imports: vec![],
            decls:   vec![],
            span:    Span { start: 0, end: 0 },
        };
        let (src, entries) = build_harness(&m, None).unwrap();
        assert!(entries.is_empty());
        assert!(src.contains("int main("));
    }

    #[test]
    fn property_params_flow_into_entry_and_dispatch() {
        let m = certo_parser::parse(
            "module A\nproperty \"commutes\"(x: Int, y: Text) { true }"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].params, vec![
            ("x".to_string(), GenType::Int),
            ("y".to_string(), GenType::Text),
        ]);
        // The generated dispatch decodes via a runtime cursor (argv_i),
        // starting at 2, and calls the wrapper with two arguments, not zero.
        assert!(src.contains("int argv_i = 2;"));
        assert!(src.contains("atoll(argv[argv_i++])"));
        assert_eq!(src.matches("argv[argv_i++]").count(), 2, "expected one decode per param");
    }

    #[test]
    fn property_without_params_dispatches_as_before() {
        let m = certo_parser::parse(
            "module A\nproperty \"holds\" { true }"
        ).unwrap();
        let (_src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].params.is_empty());
    }

    #[test]
    fn unsupported_property_param_type_is_a_clear_error() {
        // Decimal has no generator (still, unlike Int/Float/Bool/Text/List/
        // named-record/named-sum — see BACKLOG item 124).
        let m = certo_parser::parse(
            "module A\nproperty \"holds\"(items: Decimal) { true }"
        ).unwrap();
        let err = build_harness(&m, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("items"), "error should name the offending param: {msg}");
        assert!(msg.contains("holds"), "error should name the property: {msg}");
    }

    #[test]
    fn list_param_dispatch_decodes_length_prefixed_loop() {
        let m = certo_parser::parse(
            "module A\nproperty \"sums\"(xs: List<Int>) { true }"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].params, vec![("xs".to_string(), GenType::List(Box::new(GenType::Int)))]);
        assert!(src.contains("certo_list_new()"));
        assert!(src.contains("certo_list_push("));
    }

    #[test]
    fn record_param_dispatch_builds_compound_literal() {
        let m = certo_parser::parse(
            "module A\ntype Point = { x: Int, y: Int }\nproperty \"holds\"(p: Point) { true }"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0].params[0].1, GenType::Record { type_name, .. } if type_name == "Point"));
        assert!(src.contains("(Point){"));
        assert!(src.contains(".x ="));
        assert!(src.contains(".y ="));
    }

    #[test]
    fn sum_param_dispatch_reads_tag_and_calls_constructor() {
        let m = certo_parser::parse(
            "module A\ntype Shape = | Circle(radius: Float) | Empty\nproperty \"holds\"(s: Shape) { true }"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0].params[0].1, GenType::Sum { type_name, .. } if type_name == "Shape"));
        assert!(src.contains("certo_circle("), "should call the real generated constructor");
        assert!(src.contains("certo_empty"), "unit variant should reference the real generated constant");
    }

    #[test]
    fn db_test_wraps_body_in_a_real_transaction() {
        // BACKLOG item 165: dbTest previously ran its body completely
        // unwrapped — this pins that the generated C actually opens a
        // transaction (via getEnv/dbConnect/dbBegin) and rolls it back
        // (dbRollback) around the user's own body, not just labeling the
        // test kind cosmetically.
        let m = certo_parser::parse(
            "module A\ndbTest \"user creation persists\" {\n    val n = dbExec(conn, \"insert into users(name) values('x')\", [])\n    assert(n == 1, \"expected one row inserted\")\n}"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, TestKind::Db);
        assert!(src.contains("certo_get_env"), "should read DATABASE_URL via getEnv");
        assert!(src.contains("certo_db_connect"), "should open a real connection");
        assert!(src.contains("certo_db_begin"), "should begin a real transaction");
        assert!(src.contains("certo_db_rollback"), "should roll back after the body");
        // "certo_db_commit(" appears exactly once — the runtime's own
        // function *definition* (always linked in once DB support is
        // pulled in at all); a dbTest must never add a second occurrence
        // by actually *calling* it.
        assert_eq!(
            src.matches("certo_db_commit(").count(), 1,
            "a dbTest must never commit its own transaction"
        );
        // The user's own body (the insert + assert) must still be present,
        // not silently dropped in favor of the wrapper.
        assert!(src.contains("insert into users"), "the real test body must still run");
    }

    #[test]
    fn plain_test_and_property_are_unaffected_by_db_test_wrapping() {
        // The dbTest-specific wrapping path must not leak into ordinary
        // test/property bodies — they should compile exactly as before,
        // with no transaction machinery injected.
        let m = certo_parser::parse(
            "module A\ntest \"basic\" { assert(1 + 1 == 2, \"math broke\") }"
        ).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, TestKind::Unit);
        assert!(!src.contains("certo_db_begin"), "plain test must not get transaction wrapping");
    }

    #[test]
    fn self_referential_type_param_is_a_clear_error_not_a_hang() {
        let m = certo_parser::parse(
            "module A\ntype Tree = | Leaf | Node(Tree, Tree)\nproperty \"holds\"(t: Tree) { true }"
        ).unwrap();
        let err = build_harness(&m, None).unwrap_err();
        assert!(err.to_string().contains("t"));
    }

    // ------------------------------------------------------------------ //
    // BACKLOG item 222 — `ruleTest`/`validatorTest` real support.
    // ------------------------------------------------------------------ //

    const VALIDATOR_SRC: &str = "\
        type Customer = { status: Text }\n\
        type Order = { total: Int }\n\
        type OE = | NotActive | OverLimit\n\
        validator V for Order errors OE {\n\
            context { customer: Customer }\n\
            rule active { require customer.status == \"active\" else OE.NotActive }\n\
            rule creditLimit { require order.total <= 100 else OE.OverLimit }\n\
        }\n";

    #[test]
    fn ordinary_test_calling_validator_validate_now_compiles() {
        // BACKLOG item 222's own independent prerequisite-bug regression
        // guard: `certo test` previously never expanded `validator`
        // declarations at all, so `V.validate(...)` was an undefined name
        // under `build_harness` — completely independent of ruleTest/
        // validatorTest support.
        let m = certo_parser::parse(&format!(
            "module A\n{VALIDATOR_SRC}\
             test \"calls validate directly\" {{\n    \
                 val r = V.validate(Order {{ total: 1 }}, VContext {{ customer: Customer {{ status: \"active\" }} }})\n    \
                 expect(r).toBeOk()\n\
             }}"
        )).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(src.contains("certo_v_validate"), "expected the real generated V_validate function\n{src}");
    }

    #[test]
    fn rule_test_produces_a_real_entry_and_calls_the_named_rule() {
        let m = certo_parser::parse(&format!(
            "module A\n{VALIDATOR_SRC}\
             ruleTest V.active \"passes\" {{\n    \
                 entity: Order {{ total: 1 }}\n    \
                 context: VContext {{ customer: Customer {{ status: \"active\" }} }}\n    \
                 expect: pass\n\
             }}"
        )).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].display_name, "passes");
        assert_eq!(entries[0].kind, TestKind::Unit);
        assert!(src.contains("certo_v_active"), "expected a call to the real generated per-rule function\n{src}");
    }

    #[test]
    fn validator_test_calls_validate_not_a_specific_rule() {
        let m = certo_parser::parse(&format!(
            "module A\n{VALIDATOR_SRC}\
             validatorTest V \"passes\" {{\n    \
                 entity: Order {{ total: 1 }}\n    \
                 context: VContext {{ customer: Customer {{ status: \"active\" }} }}\n    \
                 expect: pass\n\
             }}"
        )).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(src.contains("certo_v_validate"), "expected a call to V_validate, not a per-rule function\n{src}");
    }

    #[test]
    fn expect_fail_with_synthesizes_a_payload_equality_check() {
        let m = certo_parser::parse(&format!(
            "module A\n{VALIDATOR_SRC}\
             ruleTest V.active \"fails with NotActive\" {{\n    \
                 entity: Order {{ total: 1 }}\n    \
                 context: VContext {{ customer: Customer {{ status: \"inactive\" }} }}\n    \
                 expect: fail with OE.NotActive\n\
             }}"
        )).unwrap();
        let (src, entries) = build_harness(&m, None).unwrap();
        assert_eq!(entries.len(), 1);
        // The match-based payload check must appear (an Err(..) arm doing
        // its own `.toBe(...)` against the expected error), not just a bare
        // `.toBeErr()` with no value check.
        assert!(src.contains("NotActive"), "expected the specific expected error value to appear\n{src}");
    }

    #[test]
    fn rule_test_naming_an_unknown_validator_is_a_real_error() {
        let m = certo_parser::parse(
            "module A\nruleTest NoSuch.someRule \"bad\" {\n    entity: 1\n    context: 1\n    expect: pass\n}"
        ).unwrap();
        let err = build_harness(&m, None).unwrap_err();
        assert!(matches!(err, crate::error::TestRunnerError::UnknownValidatorRef { .. }), "expected UnknownValidatorRef, got: {err}");
    }

    #[test]
    fn rule_test_naming_an_unknown_rule_on_a_real_validator_is_a_real_error() {
        let m = certo_parser::parse(&format!(
            "module A\n{VALIDATOR_SRC}\
             ruleTest V.noSuchRule \"bad\" {{\n    entity: 1\n    context: 1\n    expect: pass\n}}"
        )).unwrap();
        let err = build_harness(&m, None).unwrap_err();
        assert!(matches!(err, crate::error::TestRunnerError::UnknownValidatorRef { .. }), "expected UnknownValidatorRef, got: {err}");
    }
}
