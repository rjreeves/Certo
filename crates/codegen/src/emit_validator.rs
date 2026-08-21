use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use certo_ast::decl::{ValidatorDecl, RuleDecl, TriggerOp, ConstraintDecl};
use certo_fmt::{fmt_expr, fmt_type};

/// All source artifacts produced for one validator declaration.
#[derive(Debug, Default)]
pub struct ValidatorOutput {
    /// Certo source for the generated context record type (empty if no context fields).
    pub context_type: String,
    /// Certo source for `Validator.validate(entity, context): Result<Unit, ErrorsType>`.
    pub validate_fn: String,
    /// Certo source for `Validator.validateAll(entity, context): List<ErrorsType>`.
    pub validate_all_fn: String,
    /// Certo source for `Validator.validateWithDb(entity): Result<Unit, ErrorsType>`.
    /// Only present when every context field declares a `loaded by` expression.
    pub validate_with_db_fn: Option<String>,
    /// SQL CREATE TRIGGER statement emitted to `dist/triggers/<name>.sql`.
    /// Only present when the validator declares a `trigger` block.
    pub trigger_sql: Option<String>,
}

impl ValidatorOutput {
    /// Concatenate all generated fragments into one Certo source string.
    pub fn to_source(&self) -> String {
        let mut out = String::new();
        if !self.context_type.is_empty() {
            out.push_str(&self.context_type);
            out.push('\n');
        }
        out.push_str(&self.validate_fn);
        out.push('\n');
        out.push_str(&self.validate_all_fn);
        if let Some(wdb) = &self.validate_with_db_fn {
            out.push('\n');
            out.push_str(wdb);
        }
        out
    }
}

/// Generate all source artifacts for a single validator declaration.
///
/// `constraints` is the module's own name → expanded-source map from
/// `build_constraint_bodies` — BACKLOG item 220. Named constraints (spec
/// 16.8) are a compile-time-only vocabulary with no runtime symbol of their
/// own (confirmed: `Decl::Constraint` is never lowered past typeck anywhere
/// in this codebase), so a rule's `require`/`else`/`overrides` expression
/// that names one must have that constraint's body inlined here — before
/// this, the generated source referenced the bare constraint name directly,
/// which type-checked (constraints hoist as a global `Ty::Bool`) but failed
/// the C compile with `use of undeclared identifier` the moment a validator
/// using one was actually run, not just checked.
pub fn emit_validator(v: &ValidatorDecl, constraints: &HashMap<String, String>) -> ValidatorOutput {
    let mut out = ValidatorOutput::default();

    let vname       = &v.name.node;
    let entity_ty   = fmt_type(&v.entity.node, 0);
    let errors_ty   = fmt_type(&v.errors.node, 0);
    let ctx_type    = format!("{}Context", vname);

    // Entity variable: lowercase first letter of entity type name.
    let entity_var  = lowercase_first(&entity_ty);
    let expand = |raw: String| substitute_constraints(&raw, constraints);

    // ---------------------------------------------------------------- //
    // Context type
    // ---------------------------------------------------------------- //
    if !v.context.is_empty() {
        let mut s = String::new();
        writeln!(s, "type {} = {{", ctx_type).unwrap();
        for field in &v.context {
            writeln!(s, "    {}: {}", field.name.node, fmt_type(&field.type_ref.node, 1)).unwrap();
        }
        writeln!(s, "}}").unwrap();
        out.context_type = s;
    }

    // ---------------------------------------------------------------- //
    // Topological order for rules
    // ---------------------------------------------------------------- //
    let ordered = topo_sort_rules(&v.rules);

    // ---------------------------------------------------------------- //
    // `overrides` — BACKLOG item 218. An `overrides`-carrying rule's own
    // `require`/`else` never independently contributes a failure to
    // `validate`/`validateAll` — confirmed against the spec's own flagship
    // example (§16.4/16.10/16.11): a normal, non-admin user submitting a
    // perfectly valid order must not be flagged e.g. `NotAuthorised` just
    // for not being an admin, so `credit_limit_admin_override`'s own
    // `require`/`else` can only ever be a skip-switch for the rule it
    // names (`within_credit_limit`), never a failure of its own. Each
    // overridden rule instead gets an extra guard: skip it while *any*
    // rule that names it via `overrides` currently has a true condition.
    // Deliberately a *direct*, one-level relationship, not a fully
    // transitive override-chain resolver (`priority`'s own role in
    // resolving conflicts among *multiple* overriders of the same rule
    // remains unconsumed — that's W0100, BACKLOG item 220, not this one).
    let mut overridden_by: HashMap<&str, Vec<String>> = HashMap::new();
    for rule in &v.rules {
        if let Some(target) = &rule.overrides {
            overridden_by.entry(target.node.as_str())
                .or_default()
                .push(expand(fmt_expr(&rule.require.node, 0)));
        }
    }
    let override_guard = |name: &str| -> Option<String> {
        let reqs = overridden_by.get(name)?;
        if reqs.is_empty() { return None; }
        Some(reqs.iter().map(|r| format!("!({})", r)).collect::<Vec<_>>().join(" and "))
    };

    // ---------------------------------------------------------------- //
    // validate — fail fast
    // ---------------------------------------------------------------- //
    {
        let sig = build_fn_sig(vname, "validate", &entity_var, &entity_ty,
            &ctx_type, !v.context.is_empty(),
            &format!("Result<Unit, {}>", errors_ty));

        // Fail-fast: a right-folded if/then/else chain in topological order.
        // Topo order guarantees an `after`-prerequisite is checked first, so if
        // it fails we return its error before reaching the dependent rule —
        // `after` gating is satisfied by ordering alone here.
        let mut expr = String::from("Ok(unit)");
        for rule in ordered.iter().rev() {
            if rule.overrides.is_some() { continue; } // pure skip-switch, never fails on its own
            let req = expand(fmt_expr(&rule.require.node, 0));
            let err = expand(fmt_expr(&rule.else_.node, 0));
            let cond = match override_guard(&rule.name.node) {
                Some(guard) => format!("{} and !({})", guard, req),
                None => format!("!({})", req),
            };
            expr = format!("if {} then Err({}) else {}", cond, err, expr);
        }
        out.validate_fn = format!("{} =\n{}\n", sig, wrap_body(&expr, &v.context));
    }

    // ---------------------------------------------------------------- //
    // validateAll — collect all errors (list concatenation)
    // ---------------------------------------------------------------- //
    {
        let sig = build_fn_sig(vname, "validateAll", &entity_var, &entity_ty,
            &ctx_type, !v.context.is_empty(),
            &format!("List<{}>", errors_ty));

        // Each rule contributes `[err]` when it fails (and its `after`
        // prerequisites' conditions hold, and no rule overriding it is
        // active), or `[]` otherwise; concatenate all.
        let req_by_name: HashMap<&str, String> = ordered.iter()
            .map(|r| (r.name.node.as_str(), expand(fmt_expr(&r.require.node, 0))))
            .collect();

        let mut parts: Vec<String> = Vec::new();
        for rule in &ordered {
            if rule.overrides.is_some() { continue; } // pure skip-switch, never fails on its own
            let req = expand(fmt_expr(&rule.require.node, 0));
            let err = expand(fmt_expr(&rule.else_.node, 0));
            let mut conds: Vec<String> = rule.after.iter()
                .filter_map(|a| req_by_name.get(a.node.as_str()))
                .map(|r| format!("({})", r))
                .collect();
            if let Some(guard) = override_guard(&rule.name.node) {
                conds.push(guard);
            }
            conds.push(format!("!({})", req));
            // BACKLOG item 218 — real Certo uses the `and` keyword, not
            // Rust's `&&` (which lexes as two separate `Amp` tokens and
            // fails to parse) — a pre-existing mistake in this exact spot
            // for `after`-gate combination too (this file's own original
            // `gates.join(" && ")`), just never triggered by any existing
            // test or a real compile: every prior test used at most one
            // `after` dependency, and `Vec::join` never inserts a
            // separator for a single-element list, so the broken
            // separator was silently never emitted until multiple
            // conditions actually needed combining here.
            let cond = conds.join(" and ");
            parts.push(format!("(if {} then [{}] else [])", cond, err));
        }

        // Fold parts with `List.concat` (not `++`, which is overloaded with text
        // concatenation and would mis-resolve for `List<Text>` error types).
        let body = if parts.is_empty() {
            format!("([]: List<{}>)", errors_ty)
        } else {
            parts.iter().rev().skip(1).fold(parts.last().unwrap().clone(), |acc, p| {
                format!("List.concat({}, {})", p, acc)
            })
        };
        out.validate_all_fn = format!("{} =\n{}\n", sig, wrap_body(&body, &v.context));
    }

    // ---------------------------------------------------------------- //
    // validateWithDb — only when every context field has `loaded by`
    // ---------------------------------------------------------------- //
    let all_have_loaded_by = !v.context.is_empty()
        && v.context.iter().all(|f| f.loaded_by.is_some());

    if all_have_loaded_by {
        let sig = format!("async fn {}.validateWithDb({}: {}): Result<Unit, {}>",
            vname, entity_var, entity_ty, errors_ty);

        let mut body = String::new();
        writeln!(body, "    db.transaction {{").unwrap();
        for field in &v.context {
            if let Some(lb) = &field.loaded_by {
                writeln!(body, "        val {} = {}?", field.name.node, fmt_expr(&lb.node, 2)).unwrap();
            }
        }
        let field_names: Vec<String> = v.context.iter().map(|f| f.name.node.clone()).collect();
        writeln!(body, "        val ctx = {} {{ {} }}", ctx_type, field_names.join(", ")).unwrap();
        writeln!(body, "        {}.validate({}, ctx)?", vname, entity_var).unwrap();
        writeln!(body, "        Ok(())").unwrap();
        writeln!(body, "    }}").unwrap();

        out.validate_with_db_fn = Some(format!("{} =\n{}\n", sig, body));
    }

    // ---------------------------------------------------------------- //
    // Trigger SQL
    // ---------------------------------------------------------------- //
    if let Some(trigger) = &v.trigger {
        let table_name = snake_case(&entity_ty);
        let op_sql = match trigger.op {
            TriggerOp::Insert => "BEFORE INSERT",
            TriggerOp::Update => "BEFORE UPDATE",
        };
        let fn_name = format!("trg_validate_{}_{}", table_name, snake_case(vname));

        let when_clause = if let Some(cond) = &trigger.condition {
            let field = &cond.field.node;
            let value = fmt_expr(&cond.value.node, 0);
            let op = match cond.op {
                certo_ast::decl::TriggerCondOp::Eq    => "!=",
                certo_ast::decl::TriggerCondOp::NotEq => "=",
            };
            format!("    IF NEW.{field} {op} '{value}' THEN RETURN NEW; END IF;\n")
        } else {
            String::new()
        };

        let mut rules_sql = String::new();
        for rule in &ordered {
            writeln!(rules_sql, "    -- Rule: {}", rule.name.node).unwrap();
            writeln!(rules_sql, "    -- (condition evaluated in application layer)").unwrap();
        }

        let sql = format!(
"-- Generated by certo build
-- Validator: {vname}
-- Entity:    {table_name}
-- Trigger:   {op_sql}

CREATE OR REPLACE FUNCTION {fn_name}()
RETURNS TRIGGER AS $$
BEGIN
{when_clause}{rules_sql}    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS {fn_name} ON {table_name};
CREATE TRIGGER {fn_name}
    {op_sql} ON {table_name}
    FOR EACH ROW
    EXECUTE FUNCTION {fn_name}();
");
        out.trigger_sql = Some(sql);
    }

    out
}

// ------------------------------------------------------------------ //
// Helpers
// ------------------------------------------------------------------ //

fn build_fn_sig(
    vname:      &str,
    method:     &str,
    entity_var: &str,
    entity_ty:  &str,
    ctx_type:   &str,
    has_ctx:    bool,
    ret:        &str,
) -> String {
    // Use an underscore-joined name (`V_validate`) so the generated source parses
    // as an ordinary function. Call sites still write `V.validate(...)`; both map
    // to the same C symbol (`certo_v_validate`) via `c_fn_name`, so they link up.
    // The context parameter is named `ctx`, not `context` — BACKLOG item 220:
    // `context` is a lexer keyword (used by the `context { ... }` block itself),
    // so naming the parameter that made every validator with a context block
    // fail to even parse (`expected identifier, found Context`) the moment it
    // was actually built/run — `certo check` never caught it since it never
    // expands validators at all.
    if has_ctx {
        format!("fn {}_{}({}: {}, ctx: {}): {}",
            vname, method, entity_var, entity_ty, ctx_type, ret)
    } else {
        format!("fn {}_{}({}: {}): {}",
            vname, method, entity_var, entity_ty, ret)
    }
}

/// Wrap a `validate`/`validateAll` body expression in a block that first
/// destructures each context field out of the `ctx` parameter — BACKLOG item
/// 220. Previously the parameter existed but its fields (`customer`, `user`,
/// ...) were never actually bound to anything a rule's `require`/`else`
/// expression could reference by name; only the whole-struct `ctx` was ever
/// in scope, so any real context-using validator failed to compile with an
/// unbound-name error the moment the (separate) `context`-keyword collision
/// above was fixed.
fn wrap_body(expr: &str, context: &[certo_ast::decl::ContextField]) -> String {
    if context.is_empty() {
        return format!("    {}", expr);
    }
    let mut out = String::from("    {\n");
    for field in context {
        writeln!(out, "        val {} = ctx.{}", field.name.node, field.name.node).unwrap();
    }
    writeln!(out, "        {}", expr).unwrap();
    out.push_str("    }");
    out
}

/// Build a name → fully-expanded, parenthesized Certo source map for every
/// module-level `constraint` declaration, resolving constraint-references-
/// constraint composition (spec 16.8) by recursive substitution.
pub fn build_constraint_bodies(constraints: &[&ConstraintDecl]) -> HashMap<String, String> {
    let by_name: HashMap<&str, &ConstraintDecl> =
        constraints.iter().map(|c| (c.name.node.as_str(), *c)).collect();
    let mut resolved: HashMap<String, String> = HashMap::new();
    let names: Vec<&str> = by_name.keys().copied().collect();
    for name in names {
        resolve_constraint(name, &by_name, &mut resolved, &mut Vec::new());
    }
    resolved
}

/// Recursively expand one named constraint's body, substituting any other
/// constraint names it references (composition), memoizing into `resolved`.
/// `stack` guards against a reference cycle — bails out to the bare name
/// rather than recursing forever; a real cycle here isn't caught by any
/// dedicated diagnostic (out of scope for this item).
fn resolve_constraint(
    name: &str,
    by_name: &HashMap<&str, &ConstraintDecl>,
    resolved: &mut HashMap<String, String>,
    stack: &mut Vec<String>,
) -> String {
    if let Some(s) = resolved.get(name) { return s.clone(); }
    let Some(c) = by_name.get(name).copied() else { return name.to_string(); };
    if stack.iter().any(|s| s == name) { return name.to_string(); }
    stack.push(name.to_string());
    let raw = fmt_expr(&c.body.node, 0);
    let expanded = expand_idents(&raw, |word| {
        if by_name.contains_key(word) {
            Some(resolve_constraint(word, by_name, resolved, stack))
        } else {
            None
        }
    });
    stack.pop();
    let wrapped = format!("({})", expanded);
    resolved.insert(name.to_string(), wrapped.clone());
    wrapped
}

/// Replace every standalone identifier in `text` that names a known
/// constraint with that constraint's own expanded, parenthesized body.
/// Skips content inside string literals and identifiers immediately
/// following `.` (a field/qualified-path segment, never a bare constraint
/// reference).
fn substitute_constraints(text: &str, resolved: &HashMap<String, String>) -> String {
    if resolved.is_empty() { return text.to_string(); }
    expand_idents(text, |word| resolved.get(word).cloned())
}

/// Shared identifier-scanning core for both constraint-composition
/// resolution and rule-body substitution: walks `text`, skipping string
/// literals verbatim, and calls `replace` on every standalone identifier
/// not immediately preceded by `.`; `replace` returns `Some(new_text)` to
/// substitute or `None` to leave the identifier as-is.
fn expand_idents(text: &str, mut replace: impl FnMut(&str) -> Option<String>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut prev_non_space: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }
                let closed = chars[i] == '"';
                i += 1;
                if closed { break; }
            }
            prev_non_space = Some('"');
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            let word: String = chars[start..i].iter().collect();
            if prev_non_space != Some('.') {
                if let Some(replacement) = replace(&word) {
                    prev_non_space = replacement.chars().last();
                    out.push_str(&replacement);
                    continue;
                }
            }
            prev_non_space = word.chars().last();
            out.push_str(&word);
            continue;
        }
        out.push(c);
        if !c.is_whitespace() { prev_non_space = Some(c); }
        i += 1;
    }
    out
}

/// Topological sort of rules respecting `after` dependencies.
/// Rules with no dependencies come first; rules with dependencies follow.
fn topo_sort_rules<'a>(rules: &'a [RuleDecl]) -> Vec<&'a RuleDecl> {
    let idx: HashMap<&str, usize> = rules.iter()
        .enumerate()
        .map(|(i, r)| (r.name.node.as_str(), i))
        .collect();

    let mut visited = vec![false; rules.len()];
    let mut order: Vec<usize> = Vec::with_capacity(rules.len());

    fn visit(i: usize, rules: &[RuleDecl], idx: &HashMap<&str, usize>,
             visited: &mut Vec<bool>, order: &mut Vec<usize>) {
        if visited[i] { return; }
        visited[i] = true;
        for after in &rules[i].after {
            if let Some(&j) = idx.get(after.node.as_str()) {
                visit(j, rules, idx, visited, order);
            }
        }
        order.push(i);
    }

    for i in 0..rules.len() {
        visit(i, rules, &idx, &mut visited, &mut order);
    }

    order.iter().map(|&i| &rules[i]).collect()
}

fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None    => String::new(),
        Some(c) => c.to_lowercase().to_string() + chars.as_str(),
    }
}

fn snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}
