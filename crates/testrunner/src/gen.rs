//! Value generation and shrinking for property-based tests (BACKLOG item 86).
//!
//! Values are generated here, in this process, with a seeded PRNG — never
//! inside the compiled test binary. Each generated value is encoded as a
//! plain command-line argument string and handed to the already-compiled
//! test binary as `argv[2..]`; the binary parses those strings back into
//! concrete C values and calls the property function once, exactly like an
//! ordinary `test { .. }` (see `harness.rs`). Shrinking works the same way:
//! re-run the binary with a smaller candidate and check the exit code. This
//! needs no changes anywhere to `certo_panic`/`abort()` — a failing case is
//! just a nonzero exit code, same signal every other test already uses.

use certo_ast::decl::FnParam;
use certo_ast::types::TypeExpr;

/// A property parameter's generatable type. A deliberately closed set for
/// this first implementation — `List<T>`/record/sum-type generation is real,
/// well-scoped future work (the dispatch below is a plain match, trivially
/// extensible), not attempted here. See BACKLOG item 86.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenType {
    Int,
    Float,
    Bool,
    Text,
}

impl GenType {
    /// Map an AST-level parameter type annotation to a `GenType`, or `None`
    /// if generation for that type isn't supported yet.
    pub fn from_type_expr(te: &TypeExpr) -> Option<GenType> {
        match te {
            TypeExpr::Named { path, args, .. } if args.is_empty() => {
                match path.segments.last()?.node.as_str() {
                    "Int"  => Some(GenType::Int),
                    "Float" => Some(GenType::Float),
                    "Bool" => Some(GenType::Bool),
                    "Text" => Some(GenType::Text),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            GenType::Int   => "Int",
            GenType::Float => "Float",
            GenType::Bool  => "Bool",
            GenType::Text  => "Text",
        }
    }
}

/// Try to map every parameter of a `property` block to a `GenType`. Returns
/// `Err(param_name)` naming the first parameter whose type isn't supported —
/// the caller turns this into a clear build-time error rather than silently
/// skipping generation for it.
pub fn param_gen_types(params: &[FnParam]) -> Result<Vec<(String, GenType)>, String> {
    params.iter().map(|p| {
        GenType::from_type_expr(&p.ty.node)
            .map(|gt| (p.name.node.clone(), gt))
            .ok_or_else(|| p.name.node.clone())
    }).collect()
}

/// A generated value, alongside its argv-ready string encoding.
#[derive(Debug, Clone, PartialEq)]
pub enum GenValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl GenValue {
    /// Encode as a single command-line argument. Passed via `Command::arg`,
    /// never through a shell, so no quoting/escaping is needed here — each
    /// value is already one distinct argv entry regardless of its contents.
    pub fn to_arg(&self) -> String {
        match self {
            GenValue::Int(n)   => n.to_string(),
            GenValue::Float(f) => format!("{:?}", f), // Rust's Debug for f64 always round-trips
            GenValue::Bool(b)  => if *b { "true".into() } else { "false".into() },
            GenValue::Text(s)  => s.clone(),
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

/// Generate one test case: one value per declared parameter type. `case_idx`
/// grows the "size" of generated values across a run (proptest/QuickCheck
/// convention) so early cases stay small and later ones explore further out.
pub fn generate_case(rng: &mut Rng, types: &[GenType], case_idx: usize, num_cases: usize) -> Vec<GenValue> {
    let size = 1 + (case_idx * 99 / num_cases.max(1));
    types.iter().map(|t| match t {
        GenType::Int   => GenValue::Int(rng.gen_int(size as i64)),
        GenType::Float => GenValue::Float(rng.gen_float(size as f64)),
        GenType::Bool  => GenValue::Bool(rng.gen_bool()),
        GenType::Text  => GenValue::Text(rng.gen_text(size.min(20))),
    }).collect()
}

/// Candidate values "smaller" than `v`, tried in order during shrinking.
/// Empty means `v` is already minimal for its type.
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

    fn named_type(name: &str) -> TypeExpr {
        let z = Span { start: 0, end: 0 };
        TypeExpr::Named {
            path: ModulePath { segments: vec![S::new(name.to_string(), z)], span: z },
            args: vec![],
            span: z,
        }
    }

    #[test]
    fn gen_type_maps_supported_names() {
        assert_eq!(GenType::from_type_expr(&named_type("Int")), Some(GenType::Int));
        assert_eq!(GenType::from_type_expr(&named_type("Float")), Some(GenType::Float));
        assert_eq!(GenType::from_type_expr(&named_type("Bool")), Some(GenType::Bool));
        assert_eq!(GenType::from_type_expr(&named_type("Text")), Some(GenType::Text));
    }

    #[test]
    fn gen_type_rejects_unsupported_names() {
        assert_eq!(GenType::from_type_expr(&named_type("List")), None);
        assert_eq!(GenType::from_type_expr(&named_type("Decimal")), None);
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

    #[test]
    fn param_gen_types_reports_first_unsupported_param_name() {
        let z = Span { start: 0, end: 0 };
        let mk = |name: &str, ty: &str| FnParam {
            name: S::new(name.to_string(), z),
            ty: S::new(named_type(ty), z),
            default: None,
            span: z,
        };
        let ok = param_gen_types(&[mk("x", "Int"), mk("y", "Text")]).unwrap();
        assert_eq!(ok, vec![("x".to_string(), GenType::Int), ("y".to_string(), GenType::Text)]);

        let err = param_gen_types(&[mk("x", "Int"), mk("items", "List")]).unwrap_err();
        assert_eq!(err, "items");
    }
}
