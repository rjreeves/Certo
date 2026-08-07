//! Value generation and shrinking for property-based tests (BACKLOG item 86,
//! extended to `List<T>`, records, and sum types by item 124).
//!
//! Values are generated here, in this process, with a seeded PRNG — never
//! inside the compiled test binary. Each generated value is encoded as a
//! sequence of argv strings (one for a scalar; a length-prefixed run for a
//! `List<T>`; a tag-prefixed run for a sum type; a flat concatenation of its
//! fields for a record) and handed to the already-compiled test binary as
//! `argv[2..]`; the binary decodes those strings back into a concrete C
//! value and calls the property function once, exactly like an ordinary
//! `test { .. }` (see `harness.rs`). Shrinking works the same way: re-run
//! the binary with a smaller candidate and check the exit code. This needs
//! no changes anywhere to `certo_panic`/`abort()` — a failing case is just a
//! nonzero exit code, same signal every other test already uses.

use std::collections::{HashMap, HashSet};

use certo_ast::decl::{Decl, FnParam, TypeBody, TypeDecl};
use certo_ast::module::Module;
use certo_ast::types::TypeExpr;

/// A property parameter's generatable type.
///
/// `List<T>`/record/sum-type support (BACKLOG item 124) is deliberately
/// scoped: a struct-shaped value (a record or a payload-carrying sum-type
/// variant) can never appear as a `List<T>`'s *direct* element type. Reason:
/// list elements are stored in a pointer-sized `void*` slot, boxed via the
/// same convention `crates/codegen/src/emit_mir.rs`'s `box_value` uses for
/// every list/tuple element in the compiler generally — `(void*)(intptr_t)(v)`
/// for anything already pointer-sized, `(void*)__certo_f2i(v)` for `Float`.
/// A real multi-field C struct passed *by value* (how records/payload sum
/// variants are represented) doesn't fit that slot and can't be cast through
/// `intptr_t` at all — confirmed this is a genuine, pre-existing gap in the
/// compiler itself (not something introduced or fixable here), so `List<Point>`
/// is reported as an unsupported parameter type rather than silently emitting
/// C that doesn't compile or corrupts memory. A record/sum type used as a
/// *field* (not a list element) is unaffected — struct fields are stored by
/// value with their own real type, no boxing involved, so nesting there is
/// fully supported (e.g. `{ tag: Text, items: List<Int> }` or a record field
/// that is itself another named record/sum type).
///
/// A self-referential type (directly or transitively containing itself) is
/// also reported as unsupported rather than attempted — bounded-depth
/// generation/shrinking for a recursive type is a genuinely different,
/// harder problem (termination, depth control) not attempted in this pass.
///
/// Anonymous inline record annotations (`{ x: Int }` used directly, not
/// through a named `type X = { ... }`) are also not supported: the compiler
/// itself represents an anonymous record type as `void*` with no real struct
/// ever emitted (`crates/codegen/src/ty_to_c.rs`: "anonymous records become
/// void* until struct is emitted") — an unrelated, pre-existing language gap,
/// not something a value generator can work around.
#[derive(Debug, Clone, PartialEq)]
pub enum GenType {
    Int,
    Float,
    Bool,
    Text,
    /// `List<T>` — `T` is never itself `Record`/`Sum` (see doc comment above).
    List(Box<GenType>),
    /// A named record type (`type X = { field: T, ... }`), fields in
    /// declared order. `type_name` is the real declared name, used to
    /// reference the C struct emitted for it.
    Record { type_name: String, fields: Vec<(String, GenType)> },
    /// A named sum type (`type X = | A(...) | B(...) | ...`), variants in
    /// declared order.
    Sum { type_name: String, variants: Vec<SumVariantGen> },
}

/// One variant of a generatable sum type: its declared name (used to
/// reference the C constructor function/constant the compiler already
/// emits for it) and its field types in declared positional order. Empty
/// `fields` means a unit variant.
#[derive(Debug, Clone, PartialEq)]
pub struct SumVariantGen {
    pub name: String,
    pub fields: Vec<GenType>,
}

/// The module's `type X = ...` declarations, keyed by name, resolved once
/// per `build_harness` call and threaded through recursively. This is *not*
/// a typeck pass — record/sum-type shapes are read directly off the AST
/// (mirroring how `crates/hir/src/lower.rs`'s `Cx` builds its own
/// `record_field_names`/`variant_field_types` the same way) since the whole
/// module the property was declared in is already available here.
pub struct TypeDecls<'a> {
    by_name: HashMap<&'a str, &'a TypeDecl>,
}

impl<'a> TypeDecls<'a> {
    pub fn from_module(module: &'a Module) -> Self {
        let by_name = module.decls.iter()
            .filter_map(|d| match &d.node {
                Decl::Type(t) => Some((t.name.node.as_str(), t)),
                _ => None,
            })
            .collect();
        TypeDecls { by_name }
    }

    fn get(&self, name: &str) -> Option<&'a TypeDecl> {
        self.by_name.get(name).copied()
    }
}

/// Resolve an AST type annotation to a `GenType`, or `None` if generation
/// for it isn't supported. `seen` tracks named types currently being
/// resolved on the current path (not a permanent visited set — removed
/// again after that type's fields are resolved) so a type referencing
/// itself, directly or transitively, is rejected rather than recursing
/// forever; a sibling field re-referencing the same type afterwards is
/// unaffected.
pub fn resolve_gen_type(te: &TypeExpr, decls: &TypeDecls, seen: &mut HashSet<String>) -> Option<GenType> {
    match te {
        TypeExpr::Named { path, args, .. } => {
            let name = path.segments.last()?.node.as_str();
            if args.is_empty() {
                match name {
                    "Int"  => return Some(GenType::Int),
                    "Float" => return Some(GenType::Float),
                    "Bool" => return Some(GenType::Bool),
                    "Text" => return Some(GenType::Text),
                    _ => {}
                }
            }
            if name == "List" && args.len() == 1 {
                let inner = resolve_gen_type(&args[0].node, decls, seen)?;
                // A struct-shaped element can't be boxed into a list slot — see
                // the `GenType` doc comment.
                if matches!(inner, GenType::Record { .. } | GenType::Sum { .. }) {
                    return None;
                }
                return Some(GenType::List(Box::new(inner)));
            }
            if args.is_empty() {
                return resolve_named(name, decls, seen);
            }
            None
        }
        // Option/Result (`T?`), tuples, function types, anonymous records,
        // raw pointers, and bare type parameters are not attempted.
        _ => None,
    }
}

/// Resolve a *named* `type X = ...` declaration (record or sum) to a
/// `GenType`. See `resolve_gen_type`'s doc comment for the cycle-guard
/// contract — `seen` is always cleaned up before returning, on every path,
/// so a failed resolution never leaves a stale entry behind.
fn resolve_named(name: &str, decls: &TypeDecls, seen: &mut HashSet<String>) -> Option<GenType> {
    if !seen.insert(name.to_string()) {
        return None; // currently being resolved on this path — a cycle
    }

    let result = match decls.get(name) {
        None => None,
        Some(decl) => match &decl.body {
            TypeBody::Record(def) => {
                let mut fields = Vec::with_capacity(def.fields.len());
                let mut ok = true;
                for f in &def.fields {
                    match resolve_gen_type(&f.ty.node, decls, seen) {
                        Some(fgt) => fields.push((f.name.node.clone(), fgt)),
                        None => { ok = false; break; }
                    }
                }
                if ok { Some(GenType::Record { type_name: name.to_string(), fields }) } else { None }
            }
            TypeBody::Sum(variants) => {
                let mut vs = Vec::with_capacity(variants.len());
                let mut ok = true;
                'variants: for v in variants {
                    let mut fields = Vec::with_capacity(v.fields.len());
                    for f in &v.fields {
                        match resolve_gen_type(&f.ty.node, decls, seen) {
                            Some(fgt) => fields.push(fgt),
                            None => { ok = false; break 'variants; }
                        }
                    }
                    vs.push(SumVariantGen { name: v.name.node.clone(), fields });
                }
                if ok { Some(GenType::Sum { type_name: name.to_string(), variants: vs }) } else { None }
            }
            TypeBody::Alias(inner) => resolve_gen_type(&inner.node, decls, seen),
        },
    };

    seen.remove(name);
    result
}

/// Try to map every parameter of a `property` block to a `GenType`. Returns
/// `Err(param_name)` naming the first parameter whose type isn't supported —
/// the caller turns this into a clear build-time error rather than silently
/// skipping generation for it.
pub fn param_gen_types(params: &[FnParam], decls: &TypeDecls) -> Result<Vec<(String, GenType)>, String> {
    params.iter().map(|p| {
        let mut seen = HashSet::new();
        resolve_gen_type(&p.ty.node, decls, &mut seen)
            .map(|gt| (p.name.node.clone(), gt))
            .ok_or_else(|| p.name.node.clone())
    }).collect()
}

/// The minimum number of argv slots a value of this type ever encodes to —
/// exact for everything except `List`/`Sum` (a list's minimum is just its
/// length prefix at zero elements; a sum type's minimum is its tag plus
/// whichever variant has the fewest fields). Used for a cheap sanity check
/// on the decoding side, not as an exact width (which is runtime-dependent
/// for anything containing a `List`).
pub fn min_argv_width(gt: &GenType) -> usize {
    match gt {
        GenType::Int | GenType::Float | GenType::Bool | GenType::Text => 1,
        GenType::List(_) => 1,
        GenType::Record { fields, .. } => fields.iter().map(|(_, t)| min_argv_width(t)).sum(),
        GenType::Sum { variants, .. } => {
            1 + variants.iter()
                .map(|v| v.fields.iter().map(min_argv_width).sum())
                .min()
                .unwrap_or(0)
        }
    }
}

/// A generated value, alongside its argv-ready string encoding.
#[derive(Debug, Clone, PartialEq)]
pub enum GenValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    List(Vec<GenValue>),
    /// Field values in the same declared order as the `GenType::Record` they
    /// were generated from.
    Record(Vec<(String, GenValue)>),
    Sum { variant_idx: usize, variant_name: String, fields: Vec<GenValue> },
}

impl GenValue {
    /// Encode as one or more command-line arguments (never through a shell,
    /// so no quoting/escaping is needed — each string is already one
    /// distinct argv entry regardless of its contents). A scalar is exactly
    /// one argument; a `List<T>` is its length followed by each element's
    /// own encoding; a record is the flat concatenation of its fields' own
    /// encodings (field count is static, known to both sides, so no count
    /// prefix is needed); a sum type is its variant index followed by that
    /// variant's fields' own encodings.
    pub fn to_args(&self) -> Vec<String> {
        match self {
            GenValue::Int(n)   => vec![n.to_string()],
            GenValue::Float(f) => vec![format!("{:?}", f)], // Rust's Debug for f64 always round-trips
            GenValue::Bool(b)  => vec![if *b { "true".into() } else { "false".into() }],
            GenValue::Text(s)  => vec![s.clone()],
            GenValue::List(items) => {
                let mut out = vec![items.len().to_string()];
                for item in items { out.extend(item.to_args()); }
                out
            }
            GenValue::Record(fields) => fields.iter().flat_map(|(_, v)| v.to_args()).collect(),
            GenValue::Sum { variant_idx, fields, .. } => {
                let mut out = vec![variant_idx.to_string()];
                for f in fields { out.extend(f.to_args()); }
                out
            }
        }
    }

    /// Human-readable form for failure reports — not the argv encoding.
    pub fn display(&self) -> String {
        match self {
            GenValue::Int(n)   => n.to_string(),
            GenValue::Float(f) => format!("{:?}", f),
            GenValue::Bool(b)  => b.to_string(),
            GenValue::Text(s)  => format!("{:?}", s),
            GenValue::List(items) => format!("[{}]", items.iter().map(|v| v.display()).collect::<Vec<_>>().join(", ")),
            GenValue::Record(fields) => format!(
                "{{{}}}",
                fields.iter().map(|(n, v)| format!("{}: {}", n, v.display())).collect::<Vec<_>>().join(", ")
            ),
            GenValue::Sum { variant_name, fields, .. } => {
                if fields.is_empty() {
                    variant_name.clone()
                } else {
                    format!("{}({})", variant_name, fields.iter().map(|v| v.display()).collect::<Vec<_>>().join(", "))
                }
            }
        }
    }
}

/// A tiny splitmix64 PRNG — deterministic given a seed, no external
/// dependency. Good enough for test-input generation (not cryptographic use).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // 0 is a valid splitmix64 seed but produces a degenerate first output
        // for some seeds; nudge it so `Rng::new(0)` is not a special case.
        Rng(seed ^ 0x9E3779B97F4A7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, bound)`. `bound == 0` always returns 0.
    fn next_below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next_u64() % bound }
    }

    fn gen_int(&mut self, magnitude: i64) -> i64 {
        let m = magnitude.max(1) as u64;
        self.next_below(2 * m + 1) as i64 - m as i64
    }

    fn gen_float(&mut self, magnitude: f64) -> f64 {
        let u = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64; // [0, 1)
        (u * 2.0 - 1.0) * magnitude
    }

    fn gen_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn gen_text(&mut self, max_len: usize) -> String {
        let len = self.next_below(max_len as u64 + 1) as usize;
        // Printable ASCII only (33..=126): keeps encoding/shrinking simple —
        // safe to byte-slice, no argv/quoting surprises. A documented scope
        // limit, not a silent gap (see BACKLOG item 86).
        (0..len).map(|_| (33 + self.next_below(94) as u8) as char).collect()
    }
}

/// Upper bound on a generated `List`'s length, so a `List<List<List<...>>>`
/// can't blow up combinatorially — matches proptest/QuickCheck's convention
/// of keeping collection sizes modest regardless of the scalar "size" dial.
const MAX_LIST_LEN: usize = 8;

/// Generate one value of `gt`, recursing into composite types. `size` grows
/// across a run the same way `generate_case` always has (bigger/later cases
/// explore further out); nested generation halves it per level so deeply
/// nested structures don't explode alongside the top-level magnitude.
fn generate_value(rng: &mut Rng, gt: &GenType, size: usize) -> GenValue {
    match gt {
        GenType::Int   => GenValue::Int(rng.gen_int(size as i64)),
        GenType::Float => GenValue::Float(rng.gen_float(size as f64)),
        GenType::Bool  => GenValue::Bool(rng.gen_bool()),
        GenType::Text  => GenValue::Text(rng.gen_text(size.min(20))),
        GenType::List(inner) => {
            let max_len = size.min(MAX_LIST_LEN);
            let len = rng.next_below(max_len as u64 + 1) as usize;
            let child_size = (size / 2).max(1);
            GenValue::List((0..len).map(|_| generate_value(rng, inner, child_size)).collect())
        }
        GenType::Record { fields, .. } => {
            GenValue::Record(fields.iter().map(|(n, fgt)| (n.clone(), generate_value(rng, fgt, size))).collect())
        }
        GenType::Sum { variants, .. } => {
            let idx = rng.next_below(variants.len() as u64) as usize;
            let child_size = (size / 2).max(1);
            let fields = variants[idx].fields.iter().map(|fgt| generate_value(rng, fgt, child_size)).collect();
            GenValue::Sum { variant_idx: idx, variant_name: variants[idx].name.clone(), fields }
        }
    }
}

/// Generate one test case: one value per declared parameter type. `case_idx`
/// grows the "size" of generated values across a run (proptest/QuickCheck
/// convention) so early cases stay small and later ones explore further out.
pub fn generate_case(rng: &mut Rng, types: &[GenType], case_idx: usize, num_cases: usize) -> Vec<GenValue> {
    let size = 1 + (case_idx * 99 / num_cases.max(1));
    types.iter().map(|t| generate_value(rng, t, size)).collect()
}

/// Candidate values "smaller" than `v`, tried in order during shrinking.
/// Empty means `v` is already minimal for its type. For composites this
/// offers *both* "drop part of the structure" and "shrink a surviving part"
/// candidates in the same pass — the caller's greedy sweep (see `shrink`)
/// re-applies this repeatedly, so across sweeps a list converges on both
/// minimal length and minimal elements, not just one or the other.
fn shrink_step(v: &GenValue) -> Vec<GenValue> {
    match v {
        GenValue::Int(n) => {
            if *n == 0 { return vec![]; }
            let mut cands = vec![GenValue::Int(0)];
            let half = n / 2;
            if half != *n { cands.push(GenValue::Int(half)); }
            cands.push(GenValue::Int(if *n > 0 { n - 1 } else { n + 1 }));
            cands
        }
        GenValue::Float(f) => {
            if *f == 0.0 { return vec![]; }
            vec![GenValue::Float(0.0), GenValue::Float(f / 2.0)]
        }
        GenValue::Bool(b) => if *b { vec![GenValue::Bool(false)] } else { vec![] },
        GenValue::Text(s) => {
            if s.is_empty() { return vec![]; }
            let mut cands = vec![GenValue::Text(String::new())];
            let half = s.len() / 2;
            cands.push(GenValue::Text(s[..half].to_string()));
            cands.push(GenValue::Text(s[1..].to_string()));
            cands.push(GenValue::Text(s[..s.len() - 1].to_string()));
            cands
        }
        GenValue::List(items) => {
            if items.is_empty() { return vec![]; }
            let mut cands = vec![GenValue::List(vec![])];
            if items.len() > 1 {
                let half = items.len() / 2;
                cands.push(GenValue::List(items[..half].to_vec()));
                cands.push(GenValue::List(items[items.len() - half..].to_vec()));
            }
            cands.push(GenValue::List(items[..items.len() - 1].to_vec()));
            cands.push(GenValue::List(items[1..].to_vec()));
            // Shrink one surviving element at a time.
            for i in 0..items.len() {
                for smaller in shrink_step(&items[i]) {
                    let mut next = items.clone();
                    next[i] = smaller;
                    cands.push(GenValue::List(next));
                }
            }
            cands
        }
        GenValue::Record(fields) => {
            let mut cands = vec![];
            for i in 0..fields.len() {
                for smaller in shrink_step(&fields[i].1) {
                    let mut next = fields.clone();
                    next[i].1 = smaller;
                    cands.push(GenValue::Record(next));
                }
            }
            cands
        }
        GenValue::Sum { variant_idx, variant_name, fields } => {
            let mut cands = vec![];
            for i in 0..fields.len() {
                for smaller in shrink_step(&fields[i]) {
                    let mut next = fields.clone();
                    next[i] = smaller;
                    cands.push(GenValue::Sum {
                        variant_idx: *variant_idx,
                        variant_name: variant_name.clone(),
                        fields: next,
                    });
                }
            }
            cands
        }
    }
}

/// Greedily shrink a known-failing tuple of values toward a smaller/simpler
/// one that still fails `still_fails`, up to `max_attempts` trials (each a
/// real subprocess spawn from the caller). Repeatedly sweeps every parameter
/// — shrinking one can unlock further shrinking of another — until a full
/// sweep makes no progress or the budget runs out.
pub fn shrink(mut values: Vec<GenValue>, mut still_fails: impl FnMut(&[GenValue]) -> bool, max_attempts: usize) -> Vec<GenValue> {
    let mut attempts = 0usize;
    let mut improved = true;
    while improved && attempts < max_attempts {
        improved = false;
        for i in 0..values.len() {
            loop {
                let mut smaller = None;
                for cand in shrink_step(&values[i]) {
                    if attempts >= max_attempts { break; }
                    attempts += 1;
                    let mut trial = values.clone();
                    trial[i] = cand.clone();
                    if still_fails(&trial) {
                        smaller = Some(cand);
                        break;
                    }
                }
                match smaller {
                    Some(cand) => { values[i] = cand; improved = true; }
                    None => break,
                }
                if attempts >= max_attempts { break; }
            }
            if attempts >= max_attempts { break; }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use certo_ast::span::{S, Span};
    use certo_ast::types::ModulePath;

    fn z() -> Span { Span { start: 0, end: 0 } }

    fn named_type(name: &str) -> TypeExpr {
        TypeExpr::Named {
            path: ModulePath { segments: vec![S::new(name.to_string(), z())], span: z() },
            args: vec![],
            span: z(),
        }
    }

    fn list_type(inner: TypeExpr) -> TypeExpr {
        TypeExpr::Named {
            path: ModulePath { segments: vec![S::new("List".to_string(), z())], span: z() },
            args: vec![S::new(inner, z())],
            span: z(),
        }
    }

    fn empty_decls() -> TypeDecls<'static> {
        TypeDecls { by_name: HashMap::new() }
    }

    #[test]
    fn gen_type_maps_supported_scalar_names() {
        let decls = empty_decls();
        let mut seen = HashSet::new();
        assert_eq!(resolve_gen_type(&named_type("Int"), &decls, &mut seen), Some(GenType::Int));
        assert_eq!(resolve_gen_type(&named_type("Float"), &decls, &mut seen), Some(GenType::Float));
        assert_eq!(resolve_gen_type(&named_type("Bool"), &decls, &mut seen), Some(GenType::Bool));
        assert_eq!(resolve_gen_type(&named_type("Text"), &decls, &mut seen), Some(GenType::Text));
    }

    #[test]
    fn gen_type_rejects_unknown_names() {
        let decls = empty_decls();
        let mut seen = HashSet::new();
        assert_eq!(resolve_gen_type(&named_type("Decimal"), &decls, &mut seen), None);
        assert_eq!(resolve_gen_type(&named_type("SomeUndeclaredType"), &decls, &mut seen), None);
    }

    #[test]
    fn list_of_scalar_resolves_recursively() {
        let decls = empty_decls();
        let mut seen = HashSet::new();
        assert_eq!(
            resolve_gen_type(&list_type(named_type("Int")), &decls, &mut seen),
            Some(GenType::List(Box::new(GenType::Int))),
        );
        // Nested lists compose.
        assert_eq!(
            resolve_gen_type(&list_type(list_type(named_type("Float"))), &decls, &mut seen),
            Some(GenType::List(Box::new(GenType::List(Box::new(GenType::Float))))),
        );
    }

    fn parse_module(src: &str) -> Module {
        certo_parser::parse(src).expect("parse error")
    }

    #[test]
    fn named_record_type_resolves_via_module_decls() {
        let m = parse_module("module A\ntype Point = { x: Int, y: Float }");
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        let gt = resolve_gen_type(&named_type("Point"), &decls, &mut seen).unwrap();
        assert_eq!(gt, GenType::Record {
            type_name: "Point".to_string(),
            fields: vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Float)],
        });
    }

    #[test]
    fn named_sum_type_resolves_via_module_decls() {
        let m = parse_module(
            "module A\ntype Shape = | Circle(radius: Float) | Square(Float) | Point"
        );
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        let gt = resolve_gen_type(&named_type("Shape"), &decls, &mut seen).unwrap();
        assert_eq!(gt, GenType::Sum {
            type_name: "Shape".to_string(),
            variants: vec![
                SumVariantGen { name: "Circle".to_string(), fields: vec![GenType::Float] },
                SumVariantGen { name: "Square".to_string(), fields: vec![GenType::Float] },
                SumVariantGen { name: "Point".to_string(), fields: vec![] },
            ],
        });
    }

    #[test]
    fn type_alias_resolves_transparently() {
        let m = parse_module("module A\ntype Age = Int");
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        assert_eq!(resolve_gen_type(&named_type("Age"), &decls, &mut seen), Some(GenType::Int));
    }

    #[test]
    fn record_field_can_be_a_named_type_or_a_list() {
        let m = parse_module(
            "module A\ntype Line = { start: Point, tags: List<Text> }\ntype Point = { x: Int, y: Int }"
        );
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        let gt = resolve_gen_type(&named_type("Line"), &decls, &mut seen).unwrap();
        assert_eq!(gt, GenType::Record {
            type_name: "Line".to_string(),
            fields: vec![
                ("start".to_string(), GenType::Record {
                    type_name: "Point".to_string(),
                    fields: vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Int)],
                }),
                ("tags".to_string(), GenType::List(Box::new(GenType::Text))),
            ],
        });
    }

    #[test]
    fn same_named_type_in_two_sibling_fields_both_resolve() {
        // Regression guard for the cycle-guard's cleanup: resolving `Point`
        // for `start` must not block resolving it again for `end`.
        let m = parse_module(
            "module A\ntype Segment = { start: Point, end: Point }\ntype Point = { x: Int, y: Int }"
        );
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        let gt = resolve_gen_type(&named_type("Segment"), &decls, &mut seen).unwrap();
        if let GenType::Record { fields, .. } = gt {
            assert_eq!(fields.len(), 2);
            assert!(matches!(&fields[0].1, GenType::Record { .. }));
            assert!(matches!(&fields[1].1, GenType::Record { .. }));
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn self_referential_type_is_rejected_not_infinite() {
        let m = parse_module("module A\ntype Tree = | Leaf | Node(Tree, Tree)");
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        assert_eq!(resolve_gen_type(&named_type("Tree"), &decls, &mut seen), None);
    }

    #[test]
    fn list_of_record_is_rejected_struct_cant_box_into_list_slot() {
        let m = parse_module("module A\ntype Point = { x: Int, y: Int }");
        let decls = TypeDecls::from_module(&m);
        let mut seen = HashSet::new();
        assert_eq!(resolve_gen_type(&list_type(named_type("Point")), &decls, &mut seen), None);
    }

    #[test]
    fn param_gen_types_reports_first_unsupported_param_name() {
        let mk = |name: &str, ty: TypeExpr| FnParam {
            name: S::new(name.to_string(), z()),
            ty: S::new(ty, z()),
            default: None,
            span: z(),
        };
        let decls = empty_decls();
        let ok = param_gen_types(&[mk("x", named_type("Int")), mk("y", named_type("Text"))], &decls).unwrap();
        assert_eq!(ok, vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Text)]);

        let err = param_gen_types(&[mk("x", named_type("Int")), mk("items", named_type("Decimal"))], &decls).unwrap_err();
        assert_eq!(err, "items");
    }

    #[test]
    fn rng_is_deterministic_given_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let types = vec![GenType::Int, GenType::Float, GenType::Bool, GenType::Text];
        for i in 0..20 {
            assert_eq!(
                generate_case(&mut a, &types, i, 20),
                generate_case(&mut b, &types, i, 20),
            );
        }
    }

    #[test]
    fn rng_different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        let types = vec![GenType::Int];
        let ca = generate_case(&mut a, &types, 50, 100);
        let cb = generate_case(&mut b, &types, 50, 100);
        assert_ne!(ca, cb);
    }

    #[test]
    fn generated_values_stay_within_declared_magnitude() {
        let mut rng = Rng::new(7);
        let types = vec![GenType::Int, GenType::Float];
        for i in 0..100 {
            let size = 1 + (i * 99 / 100);
            if let [GenValue::Int(n), GenValue::Float(f)] = generate_case(&mut rng, &types, i, 100)[..] {
                assert!(n.unsigned_abs() as usize <= size, "Int {} exceeded size {}", n, size);
                assert!(f.abs() <= size as f64, "Float {} exceeded size {}", f, size);
            } else {
                panic!("unexpected shape");
            }
        }
    }

    #[test]
    fn generated_list_respects_max_len_and_element_type() {
        let mut rng = Rng::new(3);
        let types = vec![GenType::List(Box::new(GenType::Int))];
        for i in 0..50 {
            let case = generate_case(&mut rng, &types, i, 50);
            if let GenValue::List(items) = &case[0] {
                assert!(items.len() <= MAX_LIST_LEN);
                for it in items { assert!(matches!(it, GenValue::Int(_))); }
            } else {
                panic!("expected List");
            }
        }
    }

    #[test]
    fn generated_record_has_one_value_per_declared_field_in_order() {
        let mut rng = Rng::new(9);
        let gt = GenType::Record {
            type_name: "Point".to_string(),
            fields: vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Float)],
        };
        let v = generate_value(&mut rng, &gt, 10);
        if let GenValue::Record(fields) = v {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].0, "x");
            assert!(matches!(fields[0].1, GenValue::Int(_)));
            assert_eq!(fields[1].0, "y");
            assert!(matches!(fields[1].1, GenValue::Float(_)));
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn generated_sum_picks_a_valid_variant_index() {
        let mut rng = Rng::new(11);
        let gt = GenType::Sum {
            type_name: "Shape".to_string(),
            variants: vec![
                SumVariantGen { name: "A".to_string(), fields: vec![] },
                SumVariantGen { name: "B".to_string(), fields: vec![GenType::Int] },
            ],
        };
        for _ in 0..50 {
            let v = generate_value(&mut rng, &gt, 10);
            if let GenValue::Sum { variant_idx, variant_name, fields } = v {
                assert!(variant_idx < 2);
                if variant_idx == 0 {
                    assert_eq!(variant_name, "A");
                    assert!(fields.is_empty());
                } else {
                    assert_eq!(variant_name, "B");
                    assert_eq!(fields.len(), 1);
                }
            } else {
                panic!("expected Sum");
            }
        }
    }

    #[test]
    fn to_args_scalar_is_one_argument() {
        assert_eq!(GenValue::Int(5).to_args(), vec!["5".to_string()]);
        assert_eq!(GenValue::Bool(true).to_args(), vec!["true".to_string()]);
    }

    #[test]
    fn to_args_list_is_length_prefixed() {
        let v = GenValue::List(vec![GenValue::Int(1), GenValue::Int(2), GenValue::Int(3)]);
        assert_eq!(v.to_args(), vec!["3", "1", "2", "3"]);
        assert_eq!(GenValue::List(vec![]).to_args(), vec!["0".to_string()]);
    }

    #[test]
    fn to_args_record_is_flat_field_concatenation() {
        let v = GenValue::Record(vec![
            ("x".to_string(), GenValue::Int(1)),
            ("y".to_string(), GenValue::List(vec![GenValue::Int(2)])),
        ]);
        assert_eq!(v.to_args(), vec!["1", "1", "2"]); // x=1, then y's [len=1, elem=2]
    }

    #[test]
    fn to_args_sum_is_tag_prefixed() {
        let unit = GenValue::Sum { variant_idx: 0, variant_name: "A".into(), fields: vec![] };
        assert_eq!(unit.to_args(), vec!["0".to_string()]);
        let payload = GenValue::Sum { variant_idx: 1, variant_name: "B".into(), fields: vec![GenValue::Int(7)] };
        assert_eq!(payload.to_args(), vec!["1", "7"]);
    }

    #[test]
    fn min_argv_width_matches_scalar_and_composite_shapes() {
        assert_eq!(min_argv_width(&GenType::Int), 1);
        assert_eq!(min_argv_width(&GenType::List(Box::new(GenType::Int))), 1);
        let record = GenType::Record {
            type_name: "P".to_string(),
            fields: vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Int)],
        };
        assert_eq!(min_argv_width(&record), 2);
        let sum = GenType::Sum {
            type_name: "S".to_string(),
            variants: vec![
                SumVariantGen { name: "A".to_string(), fields: vec![] },
                SumVariantGen { name: "B".to_string(), fields: vec![GenType::Int, GenType::Int] },
            ],
        };
        assert_eq!(min_argv_width(&sum), 1); // tag + cheapest variant (A, 0 fields)
    }

    #[test]
    fn shrink_int_converges_to_zero_when_everything_fails() {
        let result = shrink(vec![GenValue::Int(973)], |_| true, 1000);
        assert_eq!(result, vec![GenValue::Int(0)]);
    }

    #[test]
    fn shrink_finds_minimal_boundary_for_a_threshold_predicate() {
        // Fails only when n >= 100 (mimics `x < 100` being the property under test).
        let result = shrink(vec![GenValue::Int(9000)], |v| {
            matches!(v[0], GenValue::Int(n) if n >= 100)
        }, 1000);
        assert_eq!(result, vec![GenValue::Int(100)]);
    }

    #[test]
    fn shrink_text_converges_to_empty_when_everything_fails() {
        let result = shrink(vec![GenValue::Text("hello world".into())], |_| true, 1000);
        assert_eq!(result, vec![GenValue::Text(String::new())]);
    }

    #[test]
    fn shrink_bool_converges_to_false_when_everything_fails() {
        let result = shrink(vec![GenValue::Bool(true)], |_| true, 1000);
        assert_eq!(result, vec![GenValue::Bool(false)]);
    }

    #[test]
    fn shrink_list_converges_to_empty_when_everything_fails() {
        let list = GenValue::List(vec![GenValue::Int(1), GenValue::Int(2), GenValue::Int(3), GenValue::Int(4)]);
        let result = shrink(vec![list], |_| true, 1000);
        assert_eq!(result, vec![GenValue::List(vec![])]);
    }

    #[test]
    fn shrink_list_finds_minimal_length_for_a_length_threshold_predicate() {
        // Fails only when the list has at least 3 elements — shrinking must
        // converge on exactly 3 elements, not just "any non-empty list" and
        // not "drop elements only, ignoring per-element shrinking".
        let list = GenValue::List((0..20).map(GenValue::Int).collect());
        let result = shrink(vec![list], |v| {
            matches!(&v[0], GenValue::List(items) if items.len() >= 3)
        }, 2000);
        if let GenValue::List(items) = &result[0] {
            assert_eq!(items.len(), 3, "expected exactly 3 elements, got {:?}", items);
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn shrink_list_also_shrinks_surviving_elements() {
        // Fails whenever the list is non-empty AND its first element >= 50 —
        // the *initial* value must genuinely satisfy this (shrink's contract
        // is "shrink a known-failing value", it never re-validates the
        // starting point itself), so both the starting list and every
        // intermediate accepted candidate must keep at least one element
        // >= 50 for the predicate to keep holding.
        let list = GenValue::List(vec![GenValue::Int(9000), GenValue::Int(9000), GenValue::Int(9000)]);
        let result = shrink(vec![list], |v| {
            matches!(&v[0], GenValue::List(items) if items.first().map(|f| matches!(f, GenValue::Int(n) if *n >= 50)).unwrap_or(false))
        }, 5000);
        // Converging to a single element at exactly the threshold proves
        // both the length got dropped to 1 (not just any non-empty list)
        // AND the surviving element got shrunk toward the boundary (not
        // left at its original magnitude) — both strategies, not just one.
        assert_eq!(result, vec![GenValue::List(vec![GenValue::Int(50)])]);
    }

    #[test]
    fn shrink_record_shrinks_each_field_independently() {
        let record = GenValue::Record(vec![
            ("x".to_string(), GenValue::Int(500)),
            ("y".to_string(), GenValue::Bool(true)),
        ]);
        let result = shrink(vec![record], |_| true, 1000);
        assert_eq!(result, vec![GenValue::Record(vec![
            ("x".to_string(), GenValue::Int(0)),
            ("y".to_string(), GenValue::Bool(false)),
        ])]);
    }

    #[test]
    fn shrink_sum_shrinks_fields_and_keeps_variant() {
        let sum = GenValue::Sum { variant_idx: 1, variant_name: "B".into(), fields: vec![GenValue::Int(500)] };
        let result = shrink(vec![sum], |_| true, 1000);
        assert_eq!(result, vec![GenValue::Sum { variant_idx: 1, variant_name: "B".into(), fields: vec![GenValue::Int(0)] }]);
    }

    #[test]
    fn shrink_respects_attempt_budget() {
        // A predicate that never lets go of `false` (only the original value
        // "fails") means shrink should give up after `max_attempts` trials
        // rather than looping forever.
        let original = GenValue::Int(973);
        let result = shrink(vec![original.clone()], |v| v[0] == original, 5);
        // With such a tiny budget it may or may not find a smaller failing
        // value before running out — the real assertion is that this
        // returns at all instead of hanging.
        let _ = result;
    }
}
