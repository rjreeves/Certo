use certo_ast::decl::Decl;
use certo_parser::parse;
use crate::{emit_module, CodegenOptions, emit_validator};

fn opts() -> CodegenOptions {
    CodegenOptions { inline_runtime: false, export_public: false, line_directives: None }
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
    let c = emit_module(&module, &CodegenOptions { inline_runtime: true, export_public: false, line_directives: None });
    assert_contains(&c, "certo_decimal_t");
    assert_contains(&c, "certo_uuid_t");
    assert_contains(&c, "CERTO_UNIT");
}

// ------------------------------------------------------------------ //
// panic() — noreturn void, not an assignable call result
// ------------------------------------------------------------------ //

#[test]
fn panic_call_in_if_branch_is_a_bare_statement() {
    // `panic(msg)` is `forall a. Text -> a` at the Certo level (usable in any
    // expression position), but `certo_panic` is a genuinely `noreturn void`
    // C function — unlike an ordinary Unit-returning Certo function, which
    // fakes a capturable int64_t zero return. Assigning a void call's result
    // is a C compile error, so the call must be emitted as a bare statement.
    let c = codegen("module A\nfn f(b: Bool): Unit = if b then panic(\"nope\") else ()");
    assert_contains(&c, "certo_panic(");
    assert_not_contains(&c, "= certo_panic(");
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
fn nullary_sum_type_emits_int_enum() {
    // All-nullary enums are plain int enums (pointer-sized) — no wrapper struct,
    // so they can flow through Result/List/tuple slots.
    let c = codegen("module A\ntype Color = | Red | Green | Blue");
    assert_contains(&c, "typedef enum {");
    assert_contains(&c, "Color_Red,");
    assert_contains(&c, "} Color;");          // the enum IS the type
    assert_not_contains(&c, "Color_tag_t");   // no struct wrapper
}

#[test]
fn payload_sum_type_emits_tagged_union() {
    // Enums with payloads keep the tagged-union struct representation.
    let c = codegen("module A\ntype Shape = | Circle(Int) | Rect(Int, Int)");
    assert_contains(&c, "Shape_tag_t");
    assert_contains(&c, "Shape_Circle,");
    assert_contains(&c, "} Shape;");
}

#[test]
fn record_field_with_generic_list_type_is_void_ptr() {
    // A field typed `List<Int>` (or any parameterized type) is a pointer
    // everywhere else in codegen (`ty_to_c(Ty::List(_)) => "void*"`); the
    // struct-emission pre-pass previously ignored the type's generic args
    // entirely and emitted the bare name (`List items;`) — an undeclared
    // C type name — instead of matching that representation.
    let c = codegen("module A\ntype Order = { items: List<Int> }");
    assert_contains(&c, "void* items;");
    assert_not_contains(&c, "List items;");
}

#[test]
fn record_field_with_generic_map_type_is_void_ptr() {
    let c = codegen("module A\ntype Roster = { scores: Map<Text, Int> }");
    assert_contains(&c, "void* scores;");
}

#[test]
fn record_field_with_option_type_is_void_ptr() {
    // `T?` fields previously emitted the FFI-header-only `certo_option_t`
    // struct — mismatched against the internal nullable-pointer Option
    // representation every other read/write of the field actually uses.
    let c = codegen("module A\ntype Person = { name: Text, nickname: Text? }");
    assert_contains(&c, "void* nickname;");
    assert_not_contains(&c, "certo_option_t");
}

#[test]
fn sum_type_variant_with_generic_payload_is_void_ptr() {
    let c = codegen("module A\ntype Bag = | Empty | Items(List<Int>)");
    assert_contains(&c, "void* f0;");
}

#[test]
fn generic_record_field_is_void_ptr_in_struct() {
    // `type Box<T> = { value: T }` — a bare type-param field's C storage is
    // `void*` (the parser never actually constructs `TypeExpr::Param`, so
    // this needs `field_c_ty`'s type-param-name lookup, not `ast_ty_to_c_str`
    // alone) — BACKLOG item 119.
    let c = codegen("module A\ntype Box<T> = { value: T }");
    assert_contains(&c, "void* value;");
    assert_not_contains(&c, "    T value;");
}

#[test]
fn generic_record_construction_boxes_type_param_field() {
    let c = codegen("module A\ntype Box<T> = { value: T }\nfn f(): Int = { val b = Box { value: 42 }\n  b.value }");
    assert_contains(&c, "malloc(sizeof("); // BoxSome on construction
}

#[test]
fn generic_record_field_read_unboxes() {
    let c = codegen("module A\ntype Box<T> = { value: T }\nfn f(): Int = { val b = Box { value: 42 }\n  b.value }");
    assert_contains(&c, "*(int64_t*)"); // UnboxSome on read, substituted to the recovered concrete type
}

#[test]
fn generic_sum_variant_field_is_void_ptr_in_struct() {
    let c = codegen("module A\ntype Bag<T> = | Empty | Items(T)");
    assert_contains(&c, "void* f0;");
}

#[test]
fn generic_sum_variant_construction_and_match_box_unbox() {
    let c = codegen(
        "module A\ntype Bag<T> = | Empty | Items(T)\nfn f(): Int = match Items(42) { Empty => 0  Items(v) => v }");
    assert_contains(&c, "malloc(sizeof("); // BoxSome on Items(42) construction
    assert_contains(&c, "*(int64_t*)");    // UnboxSome on Items(v) match-arm read
}

#[test]
fn impl_type_params_merge_with_method_type_params_in_hir() {
    // `impl<T> Secret { fn wrap(v: T): Secret<T> = ... }` — HIR has its own
    // separate `Decl::Impl` lowering pass, independent of typeck's (already
    // fixed in item 115); without merging `i.type_params` in HIR too, `T`
    // resolves to a literal (and undeclared) C type name instead of `void*`.
    let c = codegen(
        "module A\ntype Secret<T> = priv Secret(T)\nimpl<T> Secret {\n  fn wrap(v: T): Secret<T> = Secret(v)\n}");
    assert_contains(&c, "Secret certo_secret_wrap(void* _l1)");
    assert_not_contains(&c, "(T _l1)");
}

#[test]
fn option_some_is_heap_boxed() {
    // `Some(v)` must heap-box the payload (so Some(0) ≠ None and Float bits survive).
    let c = codegen("module A\nfn f(): Int? = Some(7)");
    assert_contains(&c, "malloc(sizeof(");   // typed heap box
}

#[test]
fn option_match_dereferences_payload() {
    // Matching `Some(x)` on an `Option<Float>` must read the payload as a double,
    // not treat the pointer as the value.
    let c = codegen(
        "module A\nfn f(): Float = match parseFloat(\"1.0\") { Some(x) => x  None => 0.0 }");
    assert_contains(&c, "*(double*)");   // typed dereference
}

#[test]
fn option_match_derefs_through_val_binding() {
    // The payload type must survive a `val` binding — `match q` where
    // `q = parseFloat(...)` must still dereference as a double, not int64.
    let c = codegen(
        "module A\nfn f(): Float = {\n  val q = parseFloat(\"1.0\")\n  match q { Some(x) => x  None => 0.0 }\n}");
    assert_contains(&c, "*(double*)");
}

#[test]
fn export_annotation_emits_wrapper_under_custom_name() {
    // `@export("name")` must not rename the internal function (in-module call
    // sites still use `certo_<name>`) — it adds a thin forwarding wrapper
    // under the literal custom C symbol name instead.
    let c = codegen("module A\n@export(\"my_custom_add\")\npub fn addNumbers(a: Int, b: Int): Int = a + b");
    assert_contains(&c, "certo_add_numbers(int64_t _l1, int64_t _l2)");
    assert_contains(&c, "int64_t my_custom_add(int64_t _l1, int64_t _l2) { return certo_add_numbers(_l1, _l2); }");
}

#[test]
fn export_annotation_wrapper_is_marked_export_public() {
    let module = parse("module A\n@export(\"my_custom_add\")\npub fn addNumbers(a: Int, b: Int): Int = a + b")
        .expect("parse error");
    let c = emit_module(&module, &CodegenOptions { inline_runtime: false, export_public: true, line_directives: None });
    assert_contains(&c, "CERTO_EXPORT int64_t my_custom_add(int64_t _l1, int64_t _l2)");
}

#[test]
fn datetime_typed_local_uses_certo_prefixed_c_type() {
    // A local bound from a call to a user function with an explicit `DateTime`
    // return annotation carries the real `Ty::Named("DateTime")` (unlike a
    // direct stdlib call, which HIR defaults to `Ty::Error`/`int64_t`) — so its
    // declared local type previously emitted the bare Certo name as a C type
    // (`DateTime _l1;`), an undeclared identifier, since only
    // `CertoDateTime`/`CertoDate`/`CertoDuration` typedefs exist.
    let c = codegen(
        "module A\nfn makeNow(): DateTime = DateTime.now()\nfn f(): Unit = {\n  val dt = makeNow()\n  val dt2 = dt\n  println(DateTime.toIso(dt2))\n}");
    assert_contains(&c, "CertoDateTime");
    assert_not_contains(&c, "    DateTime _l");
}

#[test]
fn duration_typed_local_uses_certo_prefixed_c_type() {
    let c = codegen(
        "module A\nfn oneDay(): Duration = Duration.days(1)\nfn f(): Unit = {\n  val d = oneDay()\n  val d2 = d\n  println(intToText(Duration.toSeconds(d2)))\n}");
    assert_contains(&c, "CertoDuration");
    assert_not_contains(&c, "    Duration _l");
}

#[test]
fn option_match_derefs_decimal_payload() {
    // parseDecimal returns Option<Decimal> — a struct-shaped payload, like
    // Result's Decimal case (BACKLOG item 114). Matching Some(x) must
    // dereference the boxed certo_decimal_t, not treat the pointer as the value.
    let c = codegen(
        "module A\nfn f(): Decimal = match parseDecimal(\"1.5\") { Some(x) => x  None => Decimal.fromInt(0) }");
    assert_contains(&c, "*(certo_decimal_t*)");
}

#[test]
fn new_collection_functions_emit_expected_call_names() {
    let c = codegen(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): Int = {\n\
         \x20 val d = List.distinct(xs)\n\
         \x20 val (a, b) = List.partition(d, (x) => x > 0)\n\
         \x20 val cs = List.chunked(a, 2)\n\
         \x20 val g = List.groupBy(b, (x) => x % 2)\n\
         \x20 List.len(b)\n\
         }");
    assert_contains(&c, "certo_list_distinct(");
    assert_contains(&c, "certo_list_partition(");
    assert_contains(&c, "certo_list_chunked(");
    assert_contains(&c, "certo_list_group_by(");
}

#[test]
fn list_get_or_panic_on_float_list_unboxes_return_value() {
    // List.getOrPanic always returns a raw void* at the C level, but HIR now
    // recovers its logical return type as the list's element type (BACKLOG
    // item 113). When that's Float, the raw pointer must be unboxed via
    // __certo_i2f rather than assigned straight into a `double` local
    // (which used to be a C type error, and before item 113 wasn't even a
    // `double` local to begin with).
    let c = codegen(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Float>): Float = List.getOrPanic(xs, 0)");
    assert_contains(&c, "__certo_i2f");
    assert_contains(&c, "double certo_f");
}

#[test]
fn list_get_or_panic_on_int_list_does_not_unbox() {
    // Int is already pointer-compatible — no bit-cast needed or wanted.
    let c = codegen(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): Int = List.getOrPanic(xs, 0)");
    assert_not_contains(&c, "__certo_i2f");
}

#[test]
fn float_callback_to_list_map_is_boxed() {
    // A lambda passed directly to List.map lifts to its own real native
    // signature (e.g. double(double)) but List.map's C runtime parameter is
    // the generic `void* (*)(void*)` (CertoFn1) — calling one through the
    // other is a calling-convention mismatch that silently corrupts Float
    // values (BACKLOG item 112). The lambda must instead be lifted with a
    // boxed void* signature, unboxing its param and boxing its return.
    let c = codegen(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Float>): Int = {\n\
         \x20 val ys = List.map(xs, (x) => x * 2.0)\n\
         \x20 List.len(ys)\n\
         }");
    assert_contains(&c, "__lam_f_0_boxed");
    assert_contains(&c, "__certo_i2f");   // unbox the incoming boxed param
    assert_contains(&c, "__certo_f2i");   // box the Float return
}

#[test]
fn int_callback_to_list_map_is_still_boxed_but_no_float_conversion() {
    // Int/Bool callbacks were already safe under the old convention (both
    // pointer-sized, general-purpose-register compatible) — confirm the new
    // boxed lift doesn't regress them: still boxed (uniform, simple), but no
    // f2i/i2f bit-cast needed since Int isn't Float.
    let c = codegen(
        "module A\nimport Stdlib.Collections.{ List }\n\
         fn f(xs: List<Int>): Int = List.len(List.map(xs, (x) => x * 2))");
    assert_contains(&c, "__lam_f_0_boxed");
    assert_not_contains(&c, "__certo_i2f");
    assert_not_contains(&c, "__certo_f2i");
}

#[test]
fn float_callback_to_user_defined_hof_is_not_boxed() {
    // The fix is deliberately narrow (BACKLOG item 112): only the specific
    // stdlib functions with a generic void* C ABI get the boxed lift.
    // A user-defined higher-order function keeps its existing, already-
    // consistent native ABI on both sides (see item 108) — boxing here
    // would be an unrelated regression, not a fix.
    let c = codegen(
        "module A\nfn twice(f: (Float) => Float, x: Float): Float = f(f(x))\n\
         fn g(): Float = twice((x) => x * 2.0, 1.0)");
    assert_not_contains(&c, "_boxed");
}

#[test]
fn map_typed_local_uses_void_star_not_undeclared_typedef() {
    // Ty::Map(k, v) used to mangle into `certo_map_{k}_{v}_t` — a typedef
    // that's never actually emitted anywhere, so any Map-typed local/param/
    // return produced an "undeclared identifier" C compile error. CertoMap*
    // is opaque regardless of K/V (same as CertoList* for List<T>), so it
    // must compile to plain `void*` like every other opaque heap type.
    // (An annotated param, not a stdlib call return, so the type is known
    // to this crate's standalone HIR pass without a full typeck seed.)
    let c = codegen("module A\nfn f(m: Map<Text, Int>): Map<Text, Int> = m");
    assert_not_contains(&c, "certo_map_text_int_t");
    assert_contains(&c, "void* certo_f(void* _l1)");
}

#[test]
fn channel_typed_local_uses_void_star_not_undeclared_typedef() {
    // Same shape of bug as Map above, for BACKLOG item 80's Channel<T>: a
    // Ty::Named{name:"Channel", args:[T]} local/param/return must map to
    // `void*` (CertoChannel* is opaque regardless of T, never a
    // per-instantiation typedef) rather than falling into the generic
    // Ty::Named arm, which assumes a real user-defined struct named
    // "Channel" exists.
    let c = codegen("module A\nfn f(ch: Channel<Int>): Channel<Int> = ch");
    assert_not_contains(&c, "certo_channel_int_t");
    assert_contains(&c, "void* certo_f(void* _l1)");
}

#[test]
fn tuple_float_element_is_bit_preserved() {
    // A Float tuple element must be bit-cast into/out of the pointer-sized slot.
    let c = codegen(
        "module A\nfn f(): Float = {\n  val (a, b) = (3.14, 2.71)\n  a + b\n}");
    assert_contains(&c, "__certo_f2i");   // box on construction
    assert_contains(&c, "__certo_i2f");   // unbox on destructure
}

#[test]
fn list_float_iteration_is_bit_preserved() {
    // Iterating a List<Float> must restore each element's bits.
    let c = codegen(
        "module A\nfn f(): Float = {\n  var t = 0.0\n  for x in [1.5, 2.5] { t = t + x }\n  t\n}");
    assert_contains(&c, "__certo_f2i");   // box each element at construction
    assert_contains(&c, "__certo_i2f");   // unbox per iteration
}

#[test]
fn result_float_payload_is_bit_preserved() {
    // A Float in a Result payload must be bit-cast into/out of the pointer-sized
    // slot, not numeric-converted (which truncated 3.14 → 3).
    let c = codegen(
        "module A\nfn mk(): Result<Float, Text> = Ok(3.14)\n\
         fn f(): Float = match mk() { Ok(v) => v  Err(e) => 0.0 }");
    assert_contains(&c, "__certo_f2i");   // bit-cast on construction
    assert_contains(&c, "__certo_i2f");   // bit-restore on extraction
}

#[test]
fn result_struct_payload_is_heap_boxed() {
    // A struct-shaped payload (a user-declared sum type here — same C
    // representation as a record) doesn't fit certo_ok/certo_err's
    // pointer-sized intptr_t slot; it must be heap-boxed on construction
    // (BoxSome-style) and dereferenced on extraction, same as Option's
    // Some(v) already does — BACKLOG item 114.
    let c = codegen(
        "module A\ntype Shape = | Circle(radius: Float)\n\
         fn mk(): Result<Shape, Text> = Ok(Circle(1.0))\n\
         fn f(): Float = match mk() {\n  Ok(s) => match s { Circle(r) => r }\n  Err(e) => 0.0\n}");
    assert_contains(&c, "malloc(sizeof(Shape))");
    assert_contains(&c, "*(Shape*)");
}

#[test]
fn result_decimal_payload_is_heap_boxed() {
    // Decimal is itself a multi-field C struct (certo_decimal_t) — same
    // boxing requirement as any user-declared struct-shaped payload.
    let c = codegen("module A\nfn mk(d: Decimal): Result<Decimal, Text> = Ok(d)");
    assert_contains(&c, "malloc(sizeof(certo_decimal_t))");
}

#[test]
fn result_int_payload_is_not_boxed() {
    // Sanity: a plain pointer-sized payload must not regress to boxing —
    // it already fits certo_ok's intptr_t slot directly.
    let c = codegen("module A\nfn mk(): Result<Int, Text> = Ok(5)");
    assert_not_contains(&c, "malloc(sizeof(int64_t))");
}

#[test]
fn try_desugar_preserves_float_payload() {
    // `e?`'s type used to always be Ty::Error, so the Ok-payload extraction
    // never bit-restored a Float — same truncation bug as match arms had,
    // via a second, independent code path (BACKLOG item 114).
    let c = codegen(
        "module A\nfn mk(): Result<Float, Text> = Ok(3.5)\n\
         fn f(): Float = mk()?");
    assert_contains(&c, "__certo_i2f");
}

#[test]
fn try_desugar_preserves_struct_payload() {
    let c = codegen(
        "module A\ntype Shape = | Circle(radius: Float)\n\
         fn mk(): Result<Shape, Text> = Ok(Circle(1.0))\n\
         fn f(): Shape = mk()?");
    assert_contains(&c, "malloc(sizeof(Shape))");
    assert_contains(&c, "*(Shape*)");
}

#[test]
fn nullary_enum_flows_through_result() {
    // The point of the int representation: a user enum can be a Result error.
    let c = codegen(
        "module A\ntype OE = | TooSmall\n\
         fn check(n: Int): Result<Int, OE> = if n > 0 then Ok(n) else Err(TooSmall)");
    // Constant is the bare tag value, not a struct initialiser.
    assert_contains(&c, "certo_too_small = OE_TooSmall;");
}

#[test]
fn sum_type_with_payload_constructor_and_match() {
    // Variant with data: the constructor name must match the call-site name
    // (`certo_circle`), and matching must read the payload via the union member
    // and positional field (`.circle.f0`).
    let c = codegen(
        "module A\ntype Shape = | Circle(Int) | Rect(Int, Int)\n\
         fn area(s: Shape): Int = match s {\n  Circle(r) => r\n  Rect(w, h) => w + h\n}");
    assert_contains(&c, "certo_circle(");   // constructor name == call-site name
    assert_contains(&c, ".circle.f0");      // payload read: union member + position
    assert_contains(&c, ".rect.f1");
    assert_not_contains(&c, "shape__circle"); // the old broken union member name
}

#[test]
fn named_field_variant_match_reads_real_field_name() {
    // A variant declared with named fields (`Circle(radius: Float)`) must be
    // read back via that same name — the struct constructor already used it
    // (`.circle = { .radius = radius }`), so a positional `.circle.f0` read
    // doesn't exist and fails to compile (BACKLOG item 111).
    let c = codegen(
        "module A\ntype Shape = | Circle(radius: Float) | Square(side: Float)\n\
         fn area(s: Shape): Float = match s {\n  Circle(r) => r\n  Square(side) => side\n}");
    assert_contains(&c, ".circle.radius");
    assert_contains(&c, ".square.side");
    assert_not_contains(&c, ".circle.f0");
    assert_not_contains(&c, ".square.f0");
}

#[test]
fn named_field_variant_mixed_positions_read_correct_names() {
    // A multi-field named variant must map each pattern position to its own
    // declared name, not fall back to a single shared name or swap fields.
    let c = codegen(
        "module A\ntype Rect = | Rect(width: Float, height: Float)\n\
         fn area(r: Rect): Float = match r {\n  Rect(w, h) => w * h\n}");
    assert_contains(&c, ".rect.width");
    assert_contains(&c, ".rect.height");
}

#[test]
fn sum_type_match_float_field_bound_as_double_not_truncated() {
    // A pattern-bound Float field (`Circle(r) => r`) must give `r`'s MIR
    // local the real `double` type, not the `Ty::Error` placeholder codegen
    // maps to `int64_t` — otherwise reading a Float payload out of the
    // union truncates it (BACKLOG item 111).
    let c = codegen(
        "module A\ntype Shape = | Circle(radius: Float) | Square(side: Float)\n\
         fn radiusOrZero(s: Shape): Float = match s {\n  Circle(r) => r\n  Square(side) => 0.0\n}");
    assert_contains(&c, "double certo_radius_or_zero");
}

#[test]
fn sum_type_match_with_computed_arm_bodies_returns_double() {
    // Arm bodies that are computed expressions (`r * r * 3.14159`) are
    // always `Ty::Error` at the HIR level (BinOp doesn't self-type there);
    // the match's own join-local type must still be recovered as `double`
    // from the arm operand, not default to `int64_t` and truncate the
    // result on return (BACKLOG item 111).
    let c = codegen(
        "module A\ntype Shape = | Circle(radius: Float) | Square(side: Float)\n\
         fn area(s: Shape): Float = match s {\n  Circle(r) => r * r * 3.14159\n  Square(side) => side * side\n}");
    assert_contains(&c, "double certo_area");
    assert_not_contains(&c, "int64_t certo_area");
}

#[test]
fn if_expr_with_computed_branch_bodies_returns_double() {
    // Same join-type-recovery fix, applied to `if`/`else` (the other branch
    // construct with the identical Ty::Error-propagation gap).
    let c = codegen(
        "module A\nfn pick(cond: Bool, a: Float, b: Float): Float = if cond then a * 2.0 else b * 3.0");
    assert_contains(&c, "double certo_pick");
    assert_not_contains(&c, "int64_t certo_pick");
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
fn calling_a_function_valued_param_casts_to_its_real_signature() {
    // BACKLOG: calling a function-valued local used to always call through the
    // opaque zero-arg certo_fn_t (its static C type), so this failed to
    // compile as C ("too many arguments"). It must now cast to the callee's
    // real, inferred signature before calling — with a leading `void*` for
    // its closure environment (BACKLOG item 140: every function value is a
    // real `{ fn, env }` closure now, not a bare pointer).
    let c = codegen("module A\nfn apply(f: (Int, Int) => Int, a: Int, b: Int): Int = f(a, b)");
    assert_contains(&c, "(int64_t(*)(void*, int64_t, int64_t))");
}

#[test]
fn calling_a_single_param_function_valued_param_casts_too() {
    let c = codegen("module A\nfn applyOne(f: Int => Int, a: Int): Int = f(a)");
    assert_contains(&c, "(int64_t(*)(void*, int64_t))");
}

#[test]
fn calling_a_named_function_directly_is_not_cast() {
    // A named top-level function's C declaration already has the real
    // signature — no cast needed, and none should be emitted.
    let c = codegen("module A\nfn add(a: Int, b: Int): Int = a + b\nfn f(): Int = add(1, 2)");
    assert_not_contains(&c, "(int64_t(*)(int64_t, int64_t))certo_add");
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
    // Generated as an underscore-named fn (parses as ordinary source); call sites
    // use `V.validate`, which links to the same C symbol.
    assert_contains(&out, "fn V_validate(");
    assert_contains(&out, "Result<Unit, OE>");
}

#[test]
fn validator_emits_validate_all_fn() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else true }\n}";
    let out = gen_validator(src);
    assert_contains(&out, "fn V_validateAll(");
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
    // Fail-fast validate checks the `after` prerequisite (`exists`) before the
    // dependent rule (`valid`), so its condition appears first.
    let src = "module A\nvalidator V for Order errors OE {\n    rule exists { require order.id > 0 else OE.Missing }\n    rule valid { after exists require order.total > 0 else OE.Invalid }\n}";
    let out = gen_validator(src);
    let pos_exists = out.find("order.id > 0").unwrap_or(usize::MAX);
    let pos_valid  = out.find("order.total > 0").unwrap_or(0);
    assert!(pos_exists < pos_valid, "prerequisite `exists` must be checked first:\n{}", out);
}

#[test]
fn validator_validate_all_collects_violations() {
    let src = "module A\nvalidator V for Order errors OE {\n    rule r { require true else OE.X }\n}";
    let out = gen_validator(src);
    // Each failing rule contributes a singleton list of its error.
    assert_contains(&out, "[OE.X]");
}

#[test]
fn generated_validator_source_parses() {
    // The whole point of wiring validators in: the generated Certo source must
    // parse (it used to emit brace-`if`/`return`, which the parser rejects).
    let src = "module A\nvalidator V for Order errors OE {\n\
        rule positive { require order.total > 0 else OE.TooSmall }\n\
        rule hasId { require order.id > 0 else OE.NoId }\n}";
    let generated = gen_validator(src);
    let wrapped = format!("module __v\ntype Order = {{ total: Int, id: Int }}\n\
        type OE = | TooSmall | NoId\n{}", generated);
    assert!(certo_parser::parse(&wrapped).is_ok(),
        "generated validator source must parse:\n{}", generated);
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
    // `b after a` — `a`'s condition must appear before `b`'s in the output.
    let src = "module A\nvalidator V for Order errors OE {\n    rule b { after a require order.bb > 0 else true }\n    rule a { require order.aa > 0 else true }\n}";
    let out = gen_validator(src);
    let pos_a = out.find("order.aa > 0").unwrap_or(usize::MAX);
    let pos_b = out.find("order.bb > 0").unwrap_or(0);
    assert!(pos_a < pos_b, "rule `a` should appear before `b`:\n{}", out);
}

// ------------------------------------------------------------------ //
// Control flow — loops and match
// ------------------------------------------------------------------ //

#[test]
fn for_loop_lowers_to_list_iteration() {
    // `for x in list` lowers to an index-counter loop over the list:
    // build list → length → indexed get → back-edge goto.
    let c = codegen("module A\nfn sum(): Int = {\n  var t = 0\n  for x in [1, 2, 3] { t = t + x }\n  t\n}");
    assert_contains(&c, "certo_list_len");
    assert_contains(&c, "certo_list_get_or_panic");
    assert_contains(&c, "goto bb");          // loop back-edge
    assert_contains(&c, "if (");             // bounds check branch
}

#[test]
fn while_loop_lowers_to_conditional_backedge() {
    // `while cond { body }` lowers to: cond block → branch → body → goto cond.
    let c = codegen("module A\nfn count(n: Int): Int = {\n  var i = 0\n  while i < n { i = i + 1 }\n  i\n}");
    assert_contains(&c, "if (");             // condition branch
    assert_contains(&c, " < ");              // the `i < n` comparison
    assert_contains(&c, "goto bb1");         // back-edge to the condition block
}

// ------------------------------------------------------------------ //
// Containers and operators
// ------------------------------------------------------------------ //

#[test]
fn list_literal_emits_list_of() {
    let c = codegen("module A\nfn f(): List<Int> = [1, 2, 3]");
    assert_contains(&c, "certo_list_of");
}

#[test]
fn string_concat_emits_text_concat() {
    let c = codegen("module A\nfn f(a: Text, b: Text): Text = a ++ b");
    assert_contains(&c, "certo_text_concat");
}

#[test]
fn extern_fn_emits_prototype_not_definition() {
    // An `extern "C"` declaration must emit a forward prototype with the correct
    // ABI (Int -> int64_t) and NO definition — the body is linked externally.
    let c = codegen(
        "module A\nextern \"C\" {\n  fn rustAdd(a: Int, b: Int): Int\n}\nfn main(): Unit = {\n  val r = rustAdd(2, 3)\n  println(intToText(r))\n}",
    );
    assert_contains(&c, "int64_t certo_rust_add(int64_t, int64_t);");
    assert_contains(&c, "certo_rust_add(2, 3)");
    // No body was generated for the extern fn.
    assert_not_contains(&c, "certo_rust_add(int64_t _l");
}

#[test]
fn fstring_int_interpolation_converts_to_text() {
    // Interpolating a non-Text value must not pass a raw int into
    // certo_text_concat (which reads it as a pointer → segfault). It must be
    // wrapped in certo_int_to_text. Covers both a named var and an inline expr.
    let c = codegen(
        "module A\nfn f(n: Int): Text = {\n  val x = n + 1\n  f\"a {x} b {n * 2}\"\n}",
    );
    assert_contains(&c, "certo_int_to_text");
}

#[test]
fn tuple_destructure_reads_by_index() {
    // `val (x, y) = (a, b)` must read tuple elements via indexed list access,
    // not C struct-field syntax (tuples are heap lists, not structs).
    let c = codegen("module A\nfn f(): Int = {\n  val (x, y) = (10, 20)\n  x + y\n}");
    assert_contains(&c, "certo_list_get_or_panic");
    assert_not_contains(&c, ".0;");   // the old broken `tmp.0` field access
    assert_not_contains(&c, ".1;");
}

#[test]
fn record_field_access_emits_dot() {
    let c = codegen("module A\ntype Pt = { x: Int, y: Int }\nfn getx(p: Pt): Int = p.x");
    assert_contains(&c, "certo_getx(");
    assert_contains(&c, ".x");
}

// ------------------------------------------------------------------ //
// Query builder (Stdlib.DbQuery)
// ------------------------------------------------------------------ //

#[test]
fn query_pipe_chain_emits_builder_calls() {
    let src = "module A\n\
        fn recent(conn: Int, mapper: List<Text?> => Int): List<Int> =\n\
        \x20   Query.from(\"Orders\")\n\
        \x20       |> Query.filter(\"status\", \"=\", \"pending\")\n\
        \x20       |> Query.orderBy(\"total\", \"desc\")\n\
        \x20       |> Query.limit(20)\n\
        \x20       |> Query.offset(5)\n\
        \x20       |> Query.list(conn, mapper)";
    let c = codegen(src);
    assert_contains(&c, "certo_query_from(");
    assert_contains(&c, "certo_query_filter(");
    assert_contains(&c, "certo_query_order_by(");
    assert_contains(&c, "certo_query_limit(");
    assert_contains(&c, "certo_query_offset(");
    assert_contains(&c, "certo_query_list(");
}

#[test]
fn query_join_emits_join_call() {
    let src = "module A\n\
        fn withCustomer(conn: Int, mapper: List<Text?> => Int): List<Int> =\n\
        \x20   Query.from(\"Orders\")\n\
        \x20       |> Query.join(\"Customers\", \"Orders.customerId\", \"Customers.id\")\n\
        \x20       |> Query.leftJoin(\"Vendors\", \"Orders.vendorId\", \"Vendors.id\")\n\
        \x20       |> Query.list(conn, mapper)";
    let c = codegen(src);
    assert_contains(&c, "certo_query_join(");
    assert_contains(&c, "certo_query_left_join(");
}

#[test]
fn query_self_join_emits_aliased_calls() {
    let src = "module A\n\
        fn withManagers(conn: Int, mapper: List<Text?> => Int): List<Int> =\n\
        \x20   Query.fromAs(\"Employees\", \"e\")\n\
        \x20       |> Query.leftJoinAs(\"Employees\", \"m\", \"e.managerId\", \"m.id\")\n\
        \x20       |> Query.filter(\"e.name\", \"=\", \"Alice\")\n\
        \x20       |> Query.list(conn, mapper)";
    let c = codegen(src);
    assert_contains(&c, "certo_query_from_as(");
    assert_contains(&c, "certo_query_left_join_as(");
}

#[test]
fn query_grouped_aggregate_emits_group_and_agg_calls() {
    let src = "module A\n\
        fn summary(conn: Int, mapper: List<Text?> => Int): List<Int> =\n\
        \x20   Query.from(\"Orders\")\n\
        \x20       |> Query.groupBy(\"status\")\n\
        \x20       |> Query.aggregate(\"count\", \"*\", \"orderCount\")\n\
        \x20       |> Query.having(\"count\", \"*\", \">\", \"0\")\n\
        \x20       |> Query.groupedList(conn, mapper)";
    let c = codegen(src);
    assert_contains(&c, "certo_query_group_by(");
    assert_contains(&c, "certo_query_aggregate(");
    assert_contains(&c, "certo_query_having(");
    assert_contains(&c, "certo_query_grouped_list(");
}

#[test]
fn query_scalar_aggregates_emit_calls() {
    let src = "module A\n\
        fn totals(conn: Int): Text? =\n\
        \x20   Query.from(\"Orders\") |> Query.sum(\"total\", conn)";
    let c = codegen(src);
    assert_contains(&c, "certo_query_sum(");
}

// ------------------------------------------------------------------ //
// Mutation builder (Stdlib.DbMutation)
// ------------------------------------------------------------------ //

#[test]
fn mutation_insert_emits_builder_calls() {
    let src = "module A\n\
        fn create(conn: Int): Int =\n\
        \x20   Mutation.insertInto(\"Orders\")\n\
        \x20       |> Mutation.set(\"status\", \"pending\")\n\
        \x20       |> Mutation.onConflict(\"id\")\n\
        \x20       |> Mutation.run(conn)";
    let c = codegen(src);
    assert_contains(&c, "certo_mutation_insert_into(");
    assert_contains(&c, "certo_mutation_set(");
    assert_contains(&c, "certo_mutation_on_conflict(");
    assert_contains(&c, "certo_mutation_run(");
}

#[test]
fn mutation_update_and_delete_emit_builder_calls() {
    let src = "module A\n\
        fn ship(conn: Int, id: Text): Int =\n\
        \x20   Mutation.updateTable(\"Orders\")\n\
        \x20       |> Mutation.set(\"status\", \"shipped\")\n\
        \x20       |> Mutation.filter(\"id\", \"=\", id)\n\
        \x20       |> Mutation.run(conn)\n\
        fn cancel(conn: Int, id: Text): Int =\n\
        \x20   Mutation.deleteFrom(\"Orders\") |> Mutation.filter(\"id\", \"=\", id) |> Mutation.run(conn)";
    let c = codegen(src);
    assert_contains(&c, "certo_mutation_update_table(");
    assert_contains(&c, "certo_mutation_filter(");
    assert_contains(&c, "certo_mutation_delete_from(");
}

#[test]
fn mutation_insert_many_emits_add_row_calls() {
    let src = "module A\n\
        fn seed(conn: Int): Int =\n\
        \x20   Mutation.insertMany(\"Orders\", [\"status\", \"total\"])\n\
        \x20       |> Mutation.addRow([\"pending\", \"19.99\"])\n\
        \x20       |> Mutation.run(conn)";
    let c = codegen(src);
    assert_contains(&c, "certo_mutation_insert_many(");
    assert_contains(&c, "certo_mutation_add_row(");
}

// ------------------------------------------------------------------ //
// Lambda lifting
// ------------------------------------------------------------------ //

#[test]
fn lambda_is_lifted_to_static_fn() {
    let c = codegen(
        "module A\nfn applyIt(x: Int, f: (Int) => Int): Int = f(x)\nfn useLam(): Int = applyIt(5) { n => n + 1 }");
    // The anonymous function is hoisted to a top-level static C function...
    assert_contains(&c, "__lam_");
    assert_contains(&c, "static");
    // ...and referenced as a function pointer at the call site.
    assert_contains(&c, "certo_apply_it");
}

// ------------------------------------------------------------------ //
// Traits / impl
// ------------------------------------------------------------------ //

#[test]
fn impl_method_emits_executable_fn() {
    // An impl method must generate a real C function and its call site must
    // reference it, so `Type.method(x)` runs.
    let c = codegen(
        "module A\ntrait Greet { fn greet(self: Text): Text }\n\
         type Person = { name: Text }\n\
         impl Greet for Person { fn greet(p: Person): Text = p.name }\n\
         fn run(p: Person): Text = Person.greet(p)");
    assert_contains(&c, "certo_person_greet");   // generated method fn
}

// ------------------------------------------------------------------ //
// Concurrency — real OS threads
// ------------------------------------------------------------------ //

#[test]
fn spawn_emits_thread_worker_and_launch() {
    // `spawn f(x)` generates a per-call-site context struct + worker function
    // and launches it on a real OS thread via the portable thread shim.
    let c = codegen(
        "module A\nfn work(n: Int): Int = n + 1\n\
         fn run(a: Int): Int = {\n  val t = spawn work(a)\n  await t\n}");
    assert_contains(&c, "__certo_thread_spawn");   // thread launch
    assert_contains(&c, "__certo_worker_");        // generated worker fn
    assert_contains(&c, "__certo_ctx_");           // generated context struct
    assert_contains(&c, "certo_work(c->a0)");      // worker calls the target
}

#[test]
fn spawn_of_non_call_body_is_lifted_not_inlined() {
    // BACKLOG item 141: a spawn body that isn't a direct call to a named
    // function (a `while` loop here — exactly the shape that used to hang
    // the program, since the old fallback inlined it into the calling
    // function's own control flow instead of running it on the worker
    // thread) must now be lifted into its own top-level function and
    // actually spawned like any other call.
    let c = codegen(
        "module A\nasync fn run(): Unit = {\n  val t = spawn { var i = 0\n while i < 3 { i = i + 1 } }\n  await t\n}");
    assert_contains(&c, "__certo_thread_spawn");
    assert_contains(&c, "__spawn_run_0");
}

#[test]
fn spawn_of_non_call_body_captures_enclosing_locals() {
    // The lifted function must receive a variable the spawn body reads from
    // its enclosing scope as a real, typed ctx-struct argument — not lose
    // it or silently misread garbage.
    let c = codegen(
        "module A\nasync fn run(limit: Int): Unit = {\n  val t = spawn { var i = 0\n while i < limit { i = i + 1 } }\n  await t\n}");
    // The captured `limit` becomes the lifted function's own parameter,
    // threaded through the worker's ctx struct exactly like an ordinary
    // spawned call's argument (`_sc->a0 = <limit's local>` at the spawn
    // site, `c->a0` read back inside the generated worker).
    assert_contains(&c, "static int64_t __spawn_run_0(int64_t");
    assert_contains(&c, "_sc->a0 = _l1;");
    assert_contains(&c, "c->a0");
}

#[test]
fn await_emits_thread_join() {
    let c = codegen(
        "module A\nfn work(n: Int): Int = n + 1\n\
         fn run(a: Int): Int = {\n  val t = spawn work(a)\n  await t\n}");
    assert_contains(&c, "__certo_thread_join");          // wait for completion
    assert_contains(&c, "->result");                     // read result via accessor
}

#[test]
fn parallel_timeout_emits_timed_join_and_panic() {
    // BACKLOG item 81: `parallel(timeout: ...)` must actually enforce the
    // timeout at runtime instead of silently dropping it.
    let c = codegen(
        "module A\nfn work(): Int = 1\n\
         fn run(): (Int, Int) = await parallel(timeout: Duration.seconds(5)) { work(), work() }");
    assert_contains(&c, "__certo_thread_join_timed");
    assert_contains(&c, "certo_panic(\"parallel(timeout: ...) block exceeded its timeout\")");
}

#[test]
fn parallel_timeout_deadline_computed_once_and_shared() {
    // The deadline (now + timeout) must be computed exactly once and reused
    // by every task's join — not recomputed per task, which would silently
    // give each task its own full timeout instead of a shared overall budget.
    let c = codegen(
        "module A\nfn work(): Int = 1\n\
         fn run(): (Int, Int) = await parallel(timeout: Duration.seconds(5)) { work(), work() }");
    // One deadline-base read (`now()` folded into the deadline expression)
    // plus one `now()` per join's remaining-time computation (2 tasks).
    assert_eq!(c.matches("certo_monotonic_millis()").count(), 3,
        "expected 1 deadline computation + 2 per-join remaining-time reads\n{c}");
    assert_eq!(c.matches("__certo_thread_join_timed").count(), 2, "one timed join per task\n{c}");
}

#[test]
fn parallel_without_timeout_does_not_use_timed_join() {
    // The runtime prelude always *defines* __certo_thread_join_timed
    // (regardless of use), so check for an actual call site, not the bare
    // substring.
    let c = codegen(
        "module A\nfn work(): Int = 1\n\
         fn run(): (Int, Int) = await parallel { work(), work() }");
    assert_not_contains(&c, "!__certo_thread_join_timed(");
    assert_contains(&c, "__certo_thread_join(((__certo_task_hdr_t*)");
}

#[test]
fn parallel_spawns_all_before_joining() {
    // `parallel { a, b }` must launch both tasks before awaiting either, so they
    // actually run concurrently. Verify both spawns precede the first join.
    let c = codegen(
        "module A\nfn work(n: Int): Int = n + 1\n\
         fn run(a: Int, b: Int): Int = {\n  \
            val t1 = spawn work(a)\n  val t2 = spawn work(b)\n  \
            val r1 = await t1\n  val r2 = await t2\n  r1 + r2\n}");
    let first_spawn = c.find("__certo_thread_spawn").expect("a spawn");
    let last_spawn  = c.rfind("__certo_thread_spawn").expect("a spawn");
    let first_join  = c.find("__certo_thread_join").expect("a join");
    assert!(first_spawn < last_spawn, "expected two distinct spawn sites");
    assert!(last_spawn < first_join, "both tasks must be spawned before the first join");
}

// ------------------------------------------------------------------ //
// State machines
// ------------------------------------------------------------------ //

fn gen_sm(src: &str) -> String {
    let module = parse(src).expect("parse error");
    let sm = module.decls.iter().find_map(|d| {
        if let Decl::StateMachine(s) = &d.node { Some(s) } else { None }
    }).expect("no statemachine decl found");
    crate::emit_state_machine(sm)
}

#[test]
fn state_machine_generates_enum_and_functions() {
    // The machine is a real struct (`state` + any accumulated event-param
    // fields), not a bare enum — BACKLOG item 82 needs somewhere to run
    // on_enter hooks / check invariants against (`self`).
    let src = "module A\nstatemachine M {\n  states:\n    Off, On\n  transitions:\n    Off -> On : turnOn()\n    On -> Off : turnOff()\n}";
    let out = gen_sm(src);
    assert_contains(&out, "type MState = | Off | On");
    assert_contains(&out, "type M = { state: MState }");
    assert_contains(&out, "fn M_new(): M = M { state: Off }"); // initial = first state
    assert_contains(&out, "fn M_turnOn(m: M): M");
    assert_contains(&out, "Off => M { ..m, state: On }");      // the transition arm
    assert_contains(&out, "fn M_isOn(m: M): Bool");
    assert_contains(&out, "fn M_state(m: M): MState = m.state");
}

#[test]
fn state_machine_groups_shared_event() {
    // `cancel` from two states must produce one function with both arms.
    let src = "module A\nstatemachine M {\n  states:\n    A, B, Dead\n  transitions:\n    A -> Dead : cancel()\n    B -> Dead : cancel()\n}";
    let out = gen_sm(src);
    assert_contains(&out, "A => M { ..m, state: Dead }");
    assert_contains(&out, "B => M { ..m, state: Dead }");
    // One combined function, not two.
    assert_eq!(out.matches("fn M_cancel(").count(), 1, "expected a single cancel fn:\n{}", out);
}

#[test]
fn state_machine_accumulates_fields_from_transition_params() {
    let src = "module A\nstatemachine M {\n  states:\n    Trial, Active\n  transitions:\n    Trial -> Active : activate(amount: Int)\n}";
    let out = gen_sm(src);
    assert_contains(&out, "type M = { state: MState, amount: Int? }");
    assert_contains(&out, "fn M_new(): M = M { state: Trial, amount: None }");
    assert_contains(&out, "Trial => M { ..m, state: Active, amount: Some(amount) }");
}

#[test]
fn state_machine_on_enter_runs_after_state_update() {
    let src = "module A\nstatemachine M {\n  states:\n    Off, On\n  transitions:\n    Off -> On : turnOn()\n\n  on_enter On: println(\"hi\")\n}";
    let out = gen_sm(src);
    let state_set_pos = out.find("state: On").expect("state update");
    let hook_pos = out.find("println(\"hi\")").expect("on_enter hook body");
    assert!(state_set_pos < hook_pos, "on_enter must run after the state field is set:\n{out}");
}

#[test]
fn state_machine_invariant_panics_with_condition_text() {
    let src = "module A\nstatemachine M {\n  states:\n    Off, On\n  transitions:\n    Off -> On : turnOn()\n\n  invariant On: 1 > 0\n}";
    let out = gen_sm(src);
    assert_contains(&out, "if !(1 > 0) then panic(\"invariant violated entering On: 1 > 0\")");
}

#[test]
fn generated_state_machine_source_parses() {
    let src = "module A\nstatemachine M {\n  states:\n    Off, On\n  transitions:\n    Off -> On : turnOn(level: Int)\n    On -> Off : turnOff()\n}";
    let generated = gen_sm(src);
    let wrapped = format!("module __m\n{}", generated);
    assert!(certo_parser::parse(&wrapped).is_ok(),
        "generated state-machine source must parse:\n{}", generated);
}

// ------------------------------------------------------------------ //
// ProcessResult opaque-handle mapping (BACKLOG item 137)
// ------------------------------------------------------------------ //

#[test]
fn process_exec_inferred_val_uses_real_pointer_type() {
    // `Process.exec`'s return type was never added to `ty_to_c`'s opaque-
    // handle table (unlike its Http/Db siblings), so *any* local bound to
    // it — even a plain, inferred `val`, not just an explicit annotation —
    // previously emitted the literal, undeclared C identifier
    // `ProcessResult` and failed to compile. Confirmed via direct repro
    // before this fix.
    let src = "module A\nfn f(): Unit [io] = {\n    val r = Process.exec(\"echo\", [\"hi\"])\n    println(ProcessResult.stdout(r))\n}";
    let out = codegen(src);
    assert_contains(&out, "CertoProcessResult* _l");
    assert_not_contains(&out, "    ProcessResult _l");
}

#[test]
fn process_exec_explicit_annotation_uses_real_pointer_type() {
    let src = "module A\nfn f(): Unit [io] = {\n    val r: ProcessResult = Process.exec(\"echo\", [\"hi\"])\n    println(ProcessResult.stdout(r))\n}";
    let out = codegen(src);
    assert_contains(&out, "CertoProcessResult* _l");
    assert_not_contains(&out, "    ProcessResult _l");
}
