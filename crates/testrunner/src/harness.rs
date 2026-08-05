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

/// Information about a single synthesized test entry point.
#[derive(Debug, Clone)]
pub struct TestEntry {
    /// Human-readable name from the source (`test "name" { ... }`).
    pub display_name: String,
    /// Kind of test block.
    pub kind: TestKind,
    /// C function name emitted for this test.
    pub c_fn_name: String,
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
/// caller can spawn sub-processes.
pub fn build_harness(module: &Module) -> (String, Vec<TestEntry>) {
    let mut augmented = module.clone();
    let mut entries: Vec<TestEntry> = Vec::new();
    let zero_span = Span { start: 0, end: 0 };

    // Synthesise a wrapper FnDecl for every test/dbTest/property block.
    for sdecl in &module.decls {
        let (display_name, body, kind) = match &sdecl.node {
            Decl::Test(t)     => (t.name.clone(), t.body.clone(), TestKind::Unit),
            Decl::DbTest(t)   => (t.name.clone(), t.body.clone(), TestKind::Db),
            Decl::Property(t) => (t.name.clone(), t.body.clone(), TestKind::Property),
            _                 => continue,
        };

        let safe  = sanitise_name(&display_name);
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
            display_name,
            kind,
            c_fn_name: fn_id.clone(),
        });
    }

    // Emit the augmented module (all user functions + test wrappers).
    // Order must match the CLI: system includes → RUNTIME_HEADER → stdlib C → user code.
    let opts = CodegenOptions { inline_runtime: false, export_public: false };
    let preamble = "#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\
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
        // Forward-declare the test function (int64_t because Ty::Error emits int64_t).
        c_src.push_str(&format!("    int64_t {mangled}(void);\n"));
        c_src.push_str(&format!(
            "    if (strcmp(target, \"{escaped}\") == 0) {{ {mangled}(); return 0; }}\n",
        ));
    }

    c_src.push_str("    fprintf(stderr, \"unknown test: %s\\n\", target);\n");
    c_src.push_str("    return 1;\n");
    c_src.push_str("}\n");

    (c_src, entries)
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
        let (src, entries) = build_harness(&m);
        assert!(entries.is_empty());
        assert!(src.contains("int main("));
    }
}
