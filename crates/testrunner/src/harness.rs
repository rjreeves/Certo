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
    decl::{Decl, FnDecl},
    module::Module,
    span::{Span, S},
};
use certo_codegen::{emit_module, CodegenOptions, c_fn_name};
use certo_stdlib::full_c_runtime;

use crate::gen::{param_gen_types, GenType};

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
pub fn build_harness(module: &Module) -> Result<(String, Vec<TestEntry>), crate::error::TestRunnerError> {
    let mut augmented = module.clone();
    let mut entries: Vec<TestEntry> = Vec::new();
    let zero_span = Span { start: 0, end: 0 };

    // Synthesise a wrapper FnDecl for every test/dbTest/property block.
    for sdecl in &module.decls {
        let (display_name, params, body, kind) = match &sdecl.node {
            Decl::Test(t)     => (t.name.clone(), vec![], t.body.clone(), TestKind::Unit),
            Decl::DbTest(t)   => (t.name.clone(), vec![], t.body.clone(), TestKind::Db),
            Decl::Property(t) => (t.name.clone(), t.params.clone(), t.body.clone(), TestKind::Property),
            _                 => continue,
        };

        let gen_params = param_gen_types(&params).map_err(|param_name| {
            crate::error::TestRunnerError::UnsupportedPropertyParamType {
                property_name: display_name.clone(),
                param_name,
            }
        })?;

        let safe  = sanitise_name(&display_name);
        let idx   = entries.len();
        let fn_id = format!("__test_{}_{}", idx, safe);

        let fn_decl = FnDecl {
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
        };
        augmented.decls.push(S::new(Decl::Fn(fn_decl), zero_span));

        entries.push(TestEntry {
            display_name,
            kind,
            c_fn_name: fn_id.clone(),
            params: gen_params,
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
    let opts = CodegenOptions { inline_runtime: false, export_public: false };
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
    c_src.push_str(&full_c_runtime());
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
            // Property test: argv[2..] carries one generated value per
            // parameter, encoded as text by the runner's driver (run.rs) and
            // decoded here into the concrete C type. A count mismatch is a
            // driver/harness bug, not a user-facing scenario — checked
            // defensively rather than silently reading past argv's end.
            let c_param_tys: Vec<&str> = e.params.iter().map(|(_, gt)| match gt {
                GenType::Int   => "int64_t",
                GenType::Float => "double",
                GenType::Bool  => "bool",
                GenType::Text  => "certo_text_t",
            }).collect();
            c_src.push_str(&format!("    int64_t {mangled}({});\n", c_param_tys.join(", ")));
            c_src.push_str(&format!(
                "    if (strcmp(target, \"{escaped}\") == 0) {{\n",
            ));
            c_src.push_str(&format!(
                "        if (argc < {}) {{ fprintf(stderr, \"{escaped}: expected {} generated argument(s)\\n\"); return 2; }}\n",
                2 + e.params.len(), e.params.len(),
            ));
            let call_args: Vec<String> = e.params.iter().enumerate().map(|(i, (_, gt))| {
                let argv_i = 2 + i;
                match gt {
                    GenType::Int   => format!("(int64_t)atoll(argv[{argv_i}])"),
                    GenType::Float => format!("atof(argv[{argv_i}])"),
                    GenType::Bool  => format!("(strcmp(argv[{argv_i}], \"true\") == 0)"),
                    GenType::Text  => format!("argv[{argv_i}]"),
                }
            }).collect();
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
        let (src, entries) = build_harness(&m).unwrap();
        assert!(entries.is_empty());
        assert!(src.contains("int main("));
    }

    #[test]
    fn property_params_flow_into_entry_and_dispatch() {
        let m = certo_parser::parse(
            "module A\nproperty \"commutes\"(x: Int, y: Text) { true }"
        ).unwrap();
        let (src, entries) = build_harness(&m).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].params, vec![
            ("x".to_string(), GenType::Int),
            ("y".to_string(), GenType::Text),
        ]);
        // The generated dispatch parses argv[2]/argv[3] into the right C
        // types and calls the wrapper with two arguments, not zero.
        assert!(src.contains("atoll(argv[2])"));
        assert!(src.contains("argv[3]"));
    }

    #[test]
    fn property_without_params_dispatches_as_before() {
        let m = certo_parser::parse(
            "module A\nproperty \"holds\" { true }"
        ).unwrap();
        let (_src, entries) = build_harness(&m).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].params.is_empty());
    }

    #[test]
    fn unsupported_property_param_type_is_a_clear_error() {
        let m = certo_parser::parse(
            "module A\nproperty \"holds\"(items: List<Int>) { true }"
        ).unwrap();
        let err = build_harness(&m).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("items"), "error should name the offending param: {msg}");
        assert!(msg.contains("holds"), "error should name the property: {msg}");
    }
}
