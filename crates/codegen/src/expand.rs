//! Expand `validator`/`statemachine` declarations into ordinary, executable
//! Certo source (real `fn`/`type` decls), by generating Certo source text
//! from them, re-parsing it, and splicing the resulting decls into the
//! caller's own module.
//!
//! Moved here from `crates/cli/src/main.rs` (BACKLOG item 222) so
//! `crates/testrunner` can call the exact same expansion `certo build`/
//! `check`/`run` already use, instead of `certo test` never expanding either
//! kind of declaration at all — previously an independent bug: any test file
//! declaring a `validator` and calling `V.validate(...)` from an ordinary
//! `test { ... }` block failed to compile under `certo test`, since nothing
//! in the test-runner's own pipeline ever turned `validator`/`statemachine`
//! into the real functions those calls need.
//!
//! Returns `Result` rather than rendering a diagnostic and calling
//! `process::exit` directly — that side-effecting behavior is CLI-specific;
//! a failure here is always a compiler bug in the generator (never a user
//! error), so each caller decides how to report it (`certo build`/`check`/
//! `run` keep their own exact previous exit-on-failure behavior;
//! `crates/testrunner` converts it into its own `TestRunnerError`).

use certo_ast::decl::Decl;
use certo_ast::module::Module;
use certo_parser::ParseError;

use crate::span_rewrite;

/// Expand each `validator { … }` declaration into executable functions
/// (`V_validate`, `V_validateAll`, one per-rule function, optional `VContext`
/// type) by generating Certo source from the validator, parsing it, and
/// splicing the decls into the module. Call sites use `V.validate(...)`,
/// which links to `V_validate` via `c_fn_name`.
///
/// `combined_src` is the growing "real file + every generated chunk so far"
/// source string a caller threads through `expand_state_machines` and this
/// function in order (BACKLOG item 223) — every spliced decl's spans are
/// rewritten (`span_rewrite::offset_decl_spans`) to index into it correctly,
/// so a type error inside generated validator code renders against the real
/// generated text instead of colliding with the real file's own span range.
///
/// `Err` means the *generated* validator source itself failed to parse — a
/// compiler bug in `emit_validator`, not something a valid Certo program can
/// trigger; the module is left unmodified in that case.
pub fn expand_validators(module: &mut Module, combined_src: &mut String) -> Result<(), (String, Vec<ParseError>)> {
    let constraints: Vec<&certo_ast::decl::ConstraintDecl> = module.decls.iter()
        .filter_map(|d| if let Decl::Constraint(c) = &d.node { Some(c) } else { None })
        .collect();
    let constraint_bodies = crate::build_constraint_bodies(&constraints);
    // BACKLOG item 243 — parallel, AST-level maps for the trigger-SQL
    // translator (`constraint_bodies` above holds already-rendered Certo
    // *source text*, only useful for the text-splicing path below).
    let constraint_asts: std::collections::HashMap<&str, &certo_ast::span::S<certo_ast::expr::Expr>> =
        constraints.iter().map(|c| (c.name.node.as_str(), &c.body)).collect();
    let temporal_asts: std::collections::HashMap<&str, &certo_ast::span::S<certo_ast::expr::Expr>> = module.decls.iter()
        .filter_map(|d| if let Decl::Temporal(t) = &d.node { Some((t.name.node.as_str(), &t.body)) } else { None })
        .collect();
    let mut generated = String::new();
    for d in &module.decls {
        if let Decl::Validator(v) = &d.node {
            generated.push_str(&crate::emit_validator(v, &constraint_bodies, &constraint_asts, &temporal_asts).to_source());
            generated.push('\n');
        }
    }
    if generated.trim().is_empty() { return Ok(()); }

    let wrapped = format!("module __validators\n{}", generated);
    match certo_parser::parse(&wrapped) {
        Ok(mut gen_module) => {
            if !combined_src.is_empty() && !combined_src.ends_with('\n') {
                combined_src.push('\n');
            }
            let offset = combined_src.len() as u32;
            for d in &mut gen_module.decls {
                span_rewrite::offset_decl_spans(d, offset);
            }
            combined_src.push_str(&wrapped);
            module.decls.extend(gen_module.decls);
            Ok(())
        }
        Err(errs) => Err((wrapped, errs)),
    }
}

/// Expand each `statemachine { … }` into a state enum plus transition /
/// predicate / accessor functions, **replacing** the original declaration.
/// After this the module contains only ordinary types and functions, so the
/// rest of the pipeline needs no special state-machine handling.
///
/// `combined_src` — see `expand_validators`'s own doc comment above (BACKLOG
/// item 223); this function must run first so its own generated chunk lands
/// first in the combined source, ahead of `expand_validators`'s.
pub fn expand_state_machines(module: &mut Module, combined_src: &mut String) -> Result<(), (String, Vec<ParseError>)> {
    let mut generated = String::new();
    let mut kept = Vec::with_capacity(module.decls.len());
    for decl in std::mem::take(&mut module.decls) {
        if let Decl::StateMachine(sm) = &decl.node {
            generated.push_str(&crate::emit_state_machine(sm));
            generated.push('\n');
            // drop the original decl — it is fully expanded
        } else {
            kept.push(decl);
        }
    }
    module.decls = kept;
    if generated.trim().is_empty() { return Ok(()); }

    let wrapped = format!("module __statemachines\n{}", generated);
    match certo_parser::parse(&wrapped) {
        Ok(mut gen_module) => {
            if !combined_src.is_empty() && !combined_src.ends_with('\n') {
                combined_src.push('\n');
            }
            let offset = combined_src.len() as u32;
            for d in &mut gen_module.decls {
                span_rewrite::offset_decl_spans(d, offset);
            }
            combined_src.push_str(&wrapped);
            module.decls.extend(gen_module.decls);
            Ok(())
        }
        Err(errs) => Err((wrapped, errs)),
    }
}
