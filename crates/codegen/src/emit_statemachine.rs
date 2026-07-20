use std::fmt::Write as FmtWrite;
use certo_ast::decl::{StateMachineDecl, FnParam};
use certo_fmt::fmt_type;

/// Generate Certo source that makes a `statemachine` executable:
///
/// - a nullary enum of the states (`type M = | A | B | …`),
/// - `M_new()` returning the initial state (first listed),
/// - one `M_<event>(m, params…)` per event — a `match` on the current state that
///   moves to the target state for a valid transition, or stays put,
/// - `M_is<State>(m): Bool` predicates,
/// - `M_state(m): M` accessor.
///
/// The machine value *is* its current state (a pointer-sized int enum), so it
/// flows through the normal function/codegen path with no special runtime.
pub fn emit_state_machine(sm: &StateMachineDecl) -> String {
    let name = &sm.name.node;
    let mut out = String::new();

    // State enum.
    let variants = sm.states.iter()
        .map(|s| format!("| {}", s.node))
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(out, "type {} = {}", name, variants).unwrap();

    // Constructor — initial state is the first one declared.
    if let Some(first) = sm.states.first() {
        writeln!(out, "fn {}_new(): {} = {}", name, name, first.node).unwrap();
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
            .map(|(from, to)| format!("{} => {}", from, to))
            .collect::<Vec<_>>()
            .join("  ");
        writeln!(out, "fn {}_{}(m: {}{}): {} = match m {{ {}  _ => m }}",
            name, event, name, param_str, name, arms_str).unwrap();
    }

    // State predicates.
    for s in &sm.states {
        writeln!(out, "fn {}_is{}(m: {}): Bool = match m {{ {} => true  _ => false }}",
            name, s.node, name, s.node).unwrap();
    }

    // Current-state accessor.
    writeln!(out, "fn {}_state(m: {}): {} = m", name, name, name).unwrap();

    out
}
