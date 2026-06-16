use certo_ast::decl::Decl;
use certo_parser::parse;
use crate::{emit_module, CodegenOptions, emit_validator};

fn opts() -> CodegenOptions {
    CodegenOptions { inline_runtime: false, export_public: false }
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
    let c = emit_module(&module, &CodegenOptions { inline_runtime: true, export_public: false });
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

// ------------------------------------------------------------------ //
// Phase 5 — Validator source generation
// ------------------------------------------------------------------ //

fn gen_validator(src: &str) -> String {
    let module = parse(src).expect("parse error");
    let v = module.decls.iter().find_map(|d| {
        if let Decl::Validator(v) = &d.node { Some(v) } else { None }
    }).expect("no validator decl found");
    emit_validator(v).to_source()
}

#[test]
fn validator_emits_validate_fn() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "fn V.validate(");
    assert_contains(&out, "Result<Unit, OE>");
}

#[test]
fn validator_emits_validate_all_fn() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "fn V.validateAll(");
    assert_contains(&out, "List<OE>");
}

#[test]
fn validator_entity_var_is_lowercase() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "order: Order");
}

#[test]
fn validator_with_context_emits_context_type() {
    let src = "module A\nvalidator V for Order errors OE {\n    context { customer: Customer }\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "type VContext = {");
    assert_contains(&out, "customer: Customer");
    assert_contains(&out, "context: VContext");
}

#[test]
fn validator_no_context_omits_context_type() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_not_contains(&out, "type VContext");
    assert_not_contains(&out, "context: VContext");
}

#[test]
fn validator_rule_require_in_output() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule nonEmpty { require order.total > 0 else OE.Empty }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "order.total > 0");
    assert_contains(&out, "OE.Empty");
}

#[test]
fn validator_after_gate_in_validate() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule exists { require order.id > 0 else OE.Missing }\n    rule valid { after exists require order.total > 0 else OE.Invalid }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "exists_passed");
}

#[test]
fn validator_validate_all_collects_violations() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else OE.X }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "violations");
    assert_contains(&out, "violations ++ [");
}

#[test]
fn validator_with_loaded_by_emits_validate_with_db() {
    let src = "module A\nvalidator V for Order errors OE {\n    context { customer: Customer loaded by db.customers.find(order.customerId) }\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "validateWithDb");
    assert_contains(&out, "db.transaction");
}

#[test]
fn validator_no_loaded_by_omits_validate_with_db() {
    let src = "module A\nvalidator V for Order errors OE {\n    context { customer: Customer }\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_not_contains(&out, "validateWithDb");
}

#[test]
fn validator_topo_order_respects_after() {
    // `b after a` — `a` must appear before `b` in the output
    let src = "module A\nvalidator V for Order errors OE {\n    rule b { after a require true else true }\n    rule a { require true else true }\n}";
    let out = gen_validator(src);
    let pos_a = out.find("a_passed").unwrap_or(usize::MAX);
    let pos_b = out.find("b_passed").unwrap_or(0);
    assert!(pos_a < pos_b, "rule `a` should appear before `b`:\n{}", out);
}
