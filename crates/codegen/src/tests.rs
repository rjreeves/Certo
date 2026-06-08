use certo_parser::parse;
use crate::{emit_module, CodegenOptions};

fn opts() -> CodegenOptions {
    CodegenOptions { inline_runtime: false }
}

fn codegen(src: &str) -> String {
    let module = parse(src).expect("parse error");
    emit_module(&module, &opts())
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(haystack.contains(needle),
        "expected to find {:?} in output:\n{}", needle, haystack);
}

fn assert_not_contains(haystack: &str, needle: &str) {
    assert!(!haystack.contains(needle),
        "did not expect {:?} in output:\n{}", needle, haystack);
}

// ------------------------------------------------------------------ //
// Header / preamble
// ------------------------------------------------------------------ //

#[test]
fn output_includes_c_headers() {
    let c = codegen("module A");
    assert_contains(&c, "#include <stdint.h>");
    assert_contains(&c, "#include <stdbool.h>");
}

#[test]
fn inline_runtime_embeds_types() {
    let module = parse("module A").expect("parse");
    let c = emit_module(&module, &CodegenOptions { inline_runtime: true });
    assert_contains(&c, "certo_decimal_t");
    assert_contains(&c, "certo_uuid_t");
    assert_contains(&c, "CERTO_UNIT");
}

// ------------------------------------------------------------------ //
// Struct emission
// ------------------------------------------------------------------ //

#[test]
fn record_type_emits_struct() {
    let c = codegen("module A\ntype Point = { x: Int, y: Int }");
    assert_contains(&c, "typedef struct {");
    assert_contains(&c, "int64_t x;");
    assert_contains(&c, "int64_t y;");
    assert_contains(&c, "} Point;");
}

#[test]
fn sum_type_emits_tagged_union() {
    let c = codegen("module A\ntype Color = | Red | Green | Blue");
    assert_contains(&c, "typedef enum {");
    assert_contains(&c, "Color_Red,");
    assert_contains(&c, "Color_tag_t");
}

// ------------------------------------------------------------------ //
// Function emission
// ------------------------------------------------------------------ //

#[test]
fn simple_fn_emits_forward_decl_and_body() {
    let c = codegen("module A\nfn answer(): Int = 42");
    assert_contains(&c, "certo_answer(");   // forward decl or body
    assert_contains(&c, "42");
}

#[test]
fn fn_with_params_emits_param_list() {
    let c = codegen("module A\nfn add(a: Int, b: Int): Int = a + b");
    assert_contains(&c, "certo_add(");
}

#[test]
fn binop_emits_c_operator() {
    let c = codegen("module A\nfn add(a: Int, b: Int): Int = a + b");
    assert_contains(&c, " + ");
}

#[test]
fn if_emits_goto_branches() {
    let c = codegen("module A\nfn pick(b: Bool): Int = if b then 1 else 2");
    assert_contains(&c, "if (");
    assert_contains(&c, "goto bb");
}

#[test]
fn match_emits_switch_structure() {
    let c = codegen("module A\nfn describe(n: Int): Text = match n {\n    0 => \"zero\"\n    _ => \"other\"\n}");
    // Match produces comparison + gotos in MIR
    assert_contains(&c, "bb");
}

#[test]
fn val_const_emits_static_global() {
    let c = codegen("module A\nval pi: Int = 3");
    assert_contains(&c, "static");
    assert_contains(&c, "3");
}

// ------------------------------------------------------------------ //
// Operator mapping
// ------------------------------------------------------------------ //

#[test]
fn eq_op_emits_double_equals() {
    let c = codegen("module A\nfn is_zero(n: Int): Bool = n == 0");
    assert_contains(&c, "==");
}

#[test]
fn and_op_emits_double_ampersand() {
    let c = codegen("module A\nfn both(a: Bool, b: Bool): Bool = a and b");
    assert_contains(&c, "&&");
}
