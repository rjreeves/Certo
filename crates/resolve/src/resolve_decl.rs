use std::collections::{HashMap, HashSet};
use certo_ast::decl::*;
use certo_ast::span::{S, Span};
use crate::scope::{ScopeChain, Res};
use crate::error::{ResolveError, ResolveErrorKind};
use crate::resolve_expr::resolve_expr;

/// Walk a declaration and resolve all names inside it.
/// Top-level names should already be pre-declared in the scope by
/// `resolve_module` before this is called (so mutual recursion works).
pub fn resolve_decl(decl: &S<Decl>, scope: &mut ScopeChain) {
    match &decl.node {
        Decl::Fn(f)  => resolve_fn(f, scope),
        Decl::Type(_) => {}    // Type bodies are structural — resolved by type checker.
        Decl::Val(v) => resolve_expr(&v.value, scope),
        Decl::Var(v) => resolve_expr(&v.value, scope),
        Decl::Trait(t) => {
            for m in &t.methods { resolve_fn(m, scope); }
        }
        Decl::Impl(i) => {
            for m in &i.methods { resolve_fn(m, scope); }
        }
        Decl::StateMachine(sm) => resolve_statemachine(sm, scope),
        Decl::Migration(_) => {}  // Migration ops resolved by the migration tool.
        Decl::View(v) => resolve_expr(&v.layout, scope),
        Decl::Form(_) => {}
        Decl::UiGenerate(_) => {}  // No expressions to resolve — just a type name and literals.
        Decl::Test(t) => {
            scope.push();
            resolve_expr(&t.body, scope);
            scope.pop();
        }
        Decl::Property(p) => {
            scope.push();
            for param in &p.params {
                scope.define(
                    &param.name.node,
                    Res::Local { span: param.name.span },
                    param.name.span,
                );
            }
            resolve_expr(&p.body, scope);
            scope.pop();
        }
        Decl::DbTest(d) => {
            scope.push();
            resolve_expr(&d.body, scope);
            scope.pop();
        }
        Decl::Validator(v)     => resolve_validator(v, decl.span, scope),
        Decl::Constraint(_)    => {}  // body checked lazily at use site in type checker
        Decl::Temporal(t)      => resolve_expr(&t.body, scope),
        Decl::RuleTest(_)      => {}  // test bodies resolved by test runner
        Decl::ValidatorTest(_) => {}  // test bodies resolved by test runner
        Decl::Import(_)        => {}
    }
}

fn resolve_fn(f: &FnDecl, scope: &mut ScopeChain) {
    scope.push();

    // Bring type parameters into scope (as builtins — type checking will validate them)
    for tp in &f.type_params {
        scope.define(
            &tp.name.node,
            Res::Builtin,
            tp.name.span,
        );
    }

    // Bring parameters into scope
    for p in &f.params {
        scope.define(
            &p.name.node,
            Res::Local { span: p.name.span },
            p.name.span,
        );
        // Default values are resolved in the outer scope (before params are bound)
        if let Some(default) = &p.default {
            // Resolve defaults against the outer scope, not the fn scope —
            // we've already pushed so pop temporarily isn't ergonomic; instead
            // we accept the slight inaccuracy and resolve here.
            resolve_expr(default, scope);
        }
    }

    if let Some(body) = &f.body {
        resolve_expr(body, scope);
    }

    scope.pop();
}

fn resolve_validator(v: &ValidatorDecl, validator_span: Span, scope: &mut ScopeChain) {
    // Build a local map of rule names so after/overrides can be validated.
    let rule_map: HashMap<&str, Span> = v.rules.iter()
        .map(|r| (r.name.node.as_str(), r.name.span))
        .collect();

    let mut has_ref_errors = false;
    for rule in &v.rules {
        for after_ref in &rule.after {
            if !rule_map.contains_key(after_ref.node.as_str()) {
                scope.errors.push(ResolveError {
                    kind: ResolveErrorKind::AfterRuleNotFound {
                        rule_name:  rule.name.node.clone(),
                        after_name: after_ref.node.clone(),
                    },
                    span: after_ref.span,
                });
                has_ref_errors = true;
            }
        }
        if let Some(ov) = &rule.overrides {
            if !rule_map.contains_key(ov.node.as_str()) {
                scope.errors.push(ResolveError {
                    kind: ResolveErrorKind::OverridesRuleNotFound {
                        rule_name:      rule.name.node.clone(),
                        overrides_name: ov.node.clone(),
                    },
                    span: ov.span,
                });
                has_ref_errors = true;
            }
        }
    }

    // Cycle detection — only meaningful when all references resolved.
    if !has_ref_errors {
        detect_rule_cycles(&v.name.node, &v.rules, validator_span, scope);
    }
}

fn detect_rule_cycles(
    validator_name: &str,
    rules:          &[RuleDecl],
    validator_span: Span,
    scope:          &mut ScopeChain,
) {
    let adj: HashMap<&str, Vec<&str>> = rules.iter()
        .map(|r| (r.name.node.as_str(), r.after.iter().map(|a| a.node.as_str()).collect()))
        .collect();

    let mut visited:   HashSet<&str> = HashSet::new();
    let mut rec_stack: Vec<&str>     = Vec::new();

    for rule in rules {
        let name = rule.name.node.as_str();
        if !visited.contains(name) {
            if let Some(cycle) = dfs_cycle(name, &adj, &mut visited, &mut rec_stack) {
                scope.errors.push(ResolveError {
                    kind: ResolveErrorKind::RuleCycle {
                        validator: validator_name.to_string(),
                        cycle,
                    },
                    span: validator_span,
                });
                return; // report only the first cycle
            }
        }
    }
}

fn dfs_cycle<'a>(
    node:      &'a str,
    adj:       &HashMap<&'a str, Vec<&'a str>>,
    visited:   &mut HashSet<&'a str>,
    rec_stack: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    visited.insert(node);
    rec_stack.push(node);

    if let Some(neighbors) = adj.get(node) {
        for &next in neighbors {
            if !visited.contains(next) {
                if let Some(cycle) = dfs_cycle(next, adj, visited, rec_stack) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(&next) {
                let start = rec_stack.iter().position(|&n| n == next).unwrap();
                let mut cycle: Vec<String> = rec_stack[start..].iter().map(|&n| n.to_string()).collect();
                cycle.push(next.to_string());
                return Some(cycle);
            }
        }
    }

    rec_stack.pop();
    None
}

fn resolve_statemachine(sm: &StateMachineDecl, scope: &mut ScopeChain) {
    // States are local names within the statemachine
    scope.push();
    for s in &sm.states {
        scope.define(&s.node, Res::Local { span: s.span }, s.span);
    }
    for hook in &sm.on_enter {
        resolve_expr(&hook.body, scope);
    }
    for inv in &sm.invariants {
        resolve_expr(&inv.cond, scope);
    }
    scope.pop();
}
