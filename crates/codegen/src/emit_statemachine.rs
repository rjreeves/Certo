use std::fmt::Write as FmtWrite;
use certo_ast::decl::{StateMachineDecl, FnParam};
use certo_ast::expr::Expr;
use certo_fmt::{fmt_type, fmt_expr};

/// Generate Certo source that makes a `statemachine` executable:
///
/// - a nullary enum of the states (`type MState = | A | B | …`),
/// - a machine struct (`type M = { state: MState, f1: T1?, f2: T2?, … }`) —
///   `f1`/`f2`/… are the union of every transition's event params, each
///   `Option`-wrapped and `None` until the transition that sets it fires,
/// - `M_new()` returning the initial state (first listed) with all fields `None`,
/// - one `M_<event>(m, params…)` per event — a `match` on the current state that
///   moves to the target state for a valid transition (running that state's
///   `on_enter` hook, if any, then panicking if any of its `invariant`s don't
///   hold), or stays put for an invalid one,
/// - `M_is<State>(m): Bool` predicates,
/// - `M_state(m): MState` accessor.
///
/// `self` is bound to the machine's new value (state already updated) while
/// evaluating `on_enter`/`invariant` bodies, matching the spec's own examples
/// (`invariant Active: self.paymentMethod.isSome()`).
///
/// The whole thing is generated as Certo *source text* and re-parsed/compiled
/// through the normal pipeline — so `on_enter`/`invariant` bodies support
/// arbitrary expressions for free, via `certo_fmt::fmt_expr` pretty-printing
/// the already-parsed AST back to source, rather than needing their own
/// AST/HIR/MIR emission path. BACKLOG item 82.
pub fn emit_state_machine(sm: &StateMachineDecl) -> String {
    let name = &sm.name.node;
    let state_ty = format!("{name}State");
    let mut out = String::new();

    // State enum.
    let variants = sm.states.iter()
        .map(|s| format!("| {}", s.node))
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(out, "type {} = {}", state_ty, variants).unwrap();

    // Machine struct: `state` plus the union of every transition's event
    // params (by name; first-seen type wins if a name repeats with a
    // different type across transitions — matches the same convention
    // `crates/codegen/src/emit_module.rs`'s bench-only statemachine path uses).
    let mut fields: Vec<(String, String)> = Vec::new();
    for t in &sm.transitions {
        for p in &t.params {
            if !fields.iter().any(|(n, _)| *n == p.name.node) {
                fields.push((p.name.node.clone(), fmt_type(&p.ty.node, 0)));
            }
        }
    }
    let field_decls: String = fields.iter()
        .map(|(n, ty)| format!(", {n}: {ty}?"))
        .collect();
    writeln!(out, "type {} = {{ state: {}{} }}", name, state_ty, field_decls).unwrap();

    // Constructor — initial state is the first one declared; all fields start `None`.
    if let Some(first) = sm.states.first() {
        let field_inits: String = fields.iter().map(|(n, _)| format!(", {n}: None")).collect();
        writeln!(out, "fn {name}_new(): {name} = {name} {{ state: {}{} }}", first.node, field_inits).unwrap();
    }

    // Group transitions by event (preserving first-seen order), collecting the
    // (from -> to) arms and the parameter list from the first occurrence.
    let mut events: Vec<(&str, &Vec<FnParam>, Vec<(&str, &str)>)> = Vec::new();
    for t in &sm.transitions {
        let ev = t.event.node.as_str();
        if let Some(slot) = events.iter_mut().find(|(e, _, _)| *e == ev) {
            slot.2.push((t.from.node.as_str(), t.to.node.as_str()));
        } else {
            events.push((ev, &t.params, vec![(t.from.node.as_str(), t.to.node.as_str())]));
        }
    }
    for (event, params, arms) in &events {
        let param_str: String = params.iter()
            .map(|p| format!(", {}: {}", p.name.node, fmt_type(&p.ty.node, 0)))
            .collect();
        let arms_str = arms.iter()
            .map(|(from, to)| format!("{} => {}", from, arm_body(name, *to, params, sm)))
            .collect::<Vec<_>>()
            .join("  ");
        writeln!(out, "fn {name}_{event}(m: {name}{param_str}): {name} = match m.state {{ {arms_str}  _ => m }}").unwrap();
    }

    // State predicates.
    for s in &sm.states {
        writeln!(out, "fn {name}_is{}(m: {name}): Bool = match m.state {{ {} => true  _ => false }}",
            s.node, s.node).unwrap();
    }

    // Current-state accessor.
    writeln!(out, "fn {name}_state(m: {name}): {state_ty} = m.state", ).unwrap();

    out
}

/// Build one `match` arm's body: update the machine (new state + this
/// transition's params set), run `to`'s `on_enter` hook if any, then check
/// `to`'s `invariant`s — panicking with the offending condition's source text
/// if one doesn't hold — and yield the updated machine.
fn arm_body(name: &str, to: &str, params: &[FnParam], sm: &StateMachineDecl) -> String {
    let field_updates: String = params.iter()
        .map(|p| format!(", {}: Some({})", p.name.node, p.name.node))
        .collect();
    let hooks: Vec<&Expr> = sm.on_enter.iter().filter(|h| h.state.node == to).map(|h| &h.body.node).collect();
    let invariants: Vec<&Expr> = sm.invariants.iter().filter(|i| i.state.node == to).map(|i| &i.cond.node).collect();

    if hooks.is_empty() && invariants.is_empty() {
        return format!("{name} {{ ..m, state: {to}{field_updates} }}");
    }

    let mut body = String::new();
    write!(body, "{{ val self = {name} {{ ..m, state: {to}{field_updates} }}  ").unwrap();
    for hook in &hooks {
        write!(body, "{}  ", fmt_expr(hook, 0)).unwrap();
    }
    for cond in &invariants {
        let src = fmt_expr(cond, 0);
        write!(body, "if !({}) then panic(\"invariant violated entering {to}: {}\")  ",
            src, escape_certo_string(&src)).unwrap();
    }
    write!(body, "self }}").unwrap();
    body
}

fn escape_certo_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
