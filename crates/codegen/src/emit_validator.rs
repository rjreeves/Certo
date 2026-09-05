use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use certo_ast::decl::{ValidatorDecl, RuleDecl, TriggerOp, ConstraintDecl};
use certo_ast::expr::{Expr, Lit, BinOp, UnOp};
use certo_ast::span::S;
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
    /// BACKLOG item 222 — one `Validator.ruleName(entity, context): Result<Unit,
    /// ErrorsType>` function per named rule, testing *that rule alone* in
    /// isolation (no `after`/`overrides`/topological-order awareness — that's
    /// what `validate`/`validateAll` are for). Needed so `ruleTest` has a real
    /// function to call; `validate`/`validateAll` above are unaffected and do
    /// not call these — they keep their own existing inlined-expression form.
    pub rule_fns: Vec<String>,
    /// Certo source for `Validator.validateWithDb(entity): Result<Unit, ErrorsType>`.
    /// Only present when every context field declares a `loaded by` expression.
    pub validate_with_db_fn: Option<String>,
    /// SQL CREATE TRIGGER statement emitted to `dist/triggers/<name>.sql`.
    /// Only present when the validator declares a `trigger` block.
    pub trigger_sql: Option<String>,
    /// BACKLOG item 243 — one message per rule the trigger SQL translator
    /// (`expr_to_sql`) couldn't fully translate, naming the rule and why —
    /// that rule stays a `-- Rule: name` comment in `trigger_sql`, silently
    /// unenforced by the installed trigger, exactly like every rule was
    /// before this item. Surfaced as a real build-time warning by callers
    /// (`crates/cli/src/main.rs`'s `collect_validator_trigger_sql`) instead
    /// of needing rediscovery via raw `psql` the way item 243 itself was found.
    pub trigger_warnings: Vec<String>,
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
        for rule_fn in &self.rule_fns {
            out.push('\n');
            out.push_str(rule_fn);
        }
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
/// `constraint_asts`/`temporal_asts` (BACKLOG item 243) are the module's own
/// name → real-AST-body maps for `constraint`/`temporal` declarations — a
/// parallel, AST-level counterpart to `constraints` (which holds already-
/// rendered *Certo source text*, only useful for the `validate`/`validateAll`
/// text-splicing path above). The trigger-SQL translator (`expr_to_sql`)
/// needs the real `Expr` tree to recurse into for inlining/constant-folding,
/// not source text to re-parse.
pub fn emit_validator(
    v: &ValidatorDecl,
    constraints: &HashMap<String, String>,
    constraint_asts: &HashMap<&str, &S<Expr>>,
    temporal_asts: &HashMap<&str, &S<Expr>>,
) -> ValidatorOutput {
    let mut out = ValidatorOutput::default();

    let vname       = &v.name.node;
    let entity_ty   = fmt_type(&v.entity.node, 0);
    let errors_ty   = fmt_type(&v.errors.node, 0);
    let ctx_type    = format!("{}Context", vname);

    // Entity variable: lowercase first letter of entity type name.
    let entity_var  = lowercase_first(&entity_ty);
    // BACKLOG item 317(b) — a `temporal` name referenced in an ordinary
    // (non-trigger) rule body (`require invoice.createdAt.age < VoidWindow`,
    // spec §16.9/§16.10's own documented shape) needs the exact same
    // text-substitution treatment already applied to `constraint`
    // references just below: `temporal` has no runtime C symbol anywhere
    // outside the SQL-trigger path (`temporal_asts`'s *other* consumer,
    // `SqlCtx`/`expr_to_sql` above), so the bare name must be inlined here
    // or it reaches generated C as an undeclared identifier.
    let temporal_bodies = build_temporal_bodies(temporal_asts);
    let expand = |raw: String| {
        let with_constraints = substitute_constraints(&raw, constraints);
        substitute_constraints(&with_constraints, &temporal_bodies)
    };

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
    // Per-rule functions — BACKLOG item 222, for `ruleTest` (spec §16.12:
    // "each rule can be tested in isolation"). Every named rule gets one,
    // including a rule that also carries `overrides` — that relationship
    // only affects how `validate`/`validateAll` *combine* rules, not what
    // the rule itself means tested alone. Iterates `v.rules` directly, not
    // `ordered`/`overridden_by` — isolation means no topological order and
    // no override-guarding, unlike `validate`/`validateAll` above.
    // ---------------------------------------------------------------- //
    for rule in &v.rules {
        let sig = build_fn_sig(vname, &rule.name.node, &entity_var, &entity_ty,
            &ctx_type, !v.context.is_empty(),
            &format!("Result<Unit, {}>", errors_ty));
        let req = expand(fmt_expr(&rule.require.node, 0));
        let err = expand(fmt_expr(&rule.else_.node, 0));
        let expr = format!("if {} then Ok(unit) else Err({})", req, err);
        out.rule_fns.push(format!("{} =\n{}\n", sig, wrap_body(&expr, &v.context)));
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
            // BACKLOG item 244 — a bare enum-variant literal (`Submitted`,
            // a single-segment `Expr::Path`) is a real SQL value and must
            // stay quoted; anything else (`OLD.status`, parsed by
            // `parse_trigger_value` as `Expr::Field { Path(["OLD"]), .. }`)
            // is a real PL/pgSQL column reference and must NOT be quoted —
            // quoting it compared `NEW.status` against the literal 9-byte
            // string `'OLD.status'` instead of the actual old row value,
            // so the gate almost never matched. `fmt_expr`'s own output
            // for `OLD.field` (`"OLD.field"`) is already valid PL/pgSQL
            // syntax verbatim — no translation needed, just don't quote it.
            let is_bare_literal = matches!(&cond.value.node,
                certo_ast::expr::Expr::Path { path, .. } if path.segments.len() == 1);
            let rendered = if is_bare_literal { format!("'{value}'") } else { value };
            format!("    IF NEW.{field} {op} {rendered} THEN RETURN NEW; END IF;\n")
        } else {
            String::new()
        };

        // BACKLOG item 243 — real per-rule SQL enforcement. Context fields
        // whose `loaded by` matches the `db.<table>.find(key)` shape (the
        // one real, structural convention `certo db pull`/item 226 both
        // already establish) resolve to a correlated subquery here; the key
        // expression itself is translated against the *entity's own*
        // fields only (a context field's `loaded by` key referencing
        // another context field is not supported — no known real use case
        // needs it, and it would risk an incorrectly-ordered subquery).
        let key_sql_ctx = SqlCtx {
            entity_var: &entity_var,
            context_subqueries: &HashMap::new(),
            constraints: constraint_asts,
            temporals: temporal_asts,
        };
        let context_subqueries: HashMap<&str, ContextSubquery> = v.context.iter()
            .filter_map(|f| {
                let lb = f.loaded_by.as_ref()?;
                let (table, key_expr) = context_field_table_and_key(&lb.node)?;
                let key_sql = expr_to_sql(key_expr, &key_sql_ctx)?;
                Some((f.name.node.as_str(), ContextSubquery { table: table.to_string(), key_sql }))
            })
            .collect();
        let sql_ctx = SqlCtx {
            entity_var: &entity_var,
            context_subqueries: &context_subqueries,
            constraints: constraint_asts,
            temporals: temporal_asts,
        };

        // Every rule's own `require`, translated once, keyed by rule name —
        // feeds both `after`-prerequisite gating (a dependent rule only
        // fires once every prerequisite's own condition holds) and
        // `overrides` guarding below. A rule missing here (translation
        // failed) can't gate any dependent rule either — conservative and
        // correct, since a partially-translated gate could silently under-
        // or over-enforce.
        let req_sql_by_name: HashMap<&str, String> = ordered.iter()
            .filter_map(|r| expr_to_sql(&r.require.node, &sql_ctx).map(|s| (r.name.node.as_str(), s)))
            .collect();

        // Mirrors `overridden_by`/`override_guard` above exactly, but
        // producing a SQL "not currently overridden" guard instead of
        // Certo source text.
        let mut overridden_by_rules: HashMap<&str, Vec<&RuleDecl>> = HashMap::new();
        for rule in &v.rules {
            if let Some(target) = &rule.overrides {
                overridden_by_rules.entry(target.node.as_str()).or_default().push(rule);
            }
        }
        // Outer `None` = not overridden at all (no guard needed). Inner
        // `None` = overridden, but at least one overriding rule's own
        // condition didn't translate — the whole gate is unknowable.
        let override_guard_sql = |name: &str| -> Option<Option<String>> {
            let rules = overridden_by_rules.get(name)?;
            if rules.is_empty() { return None; }
            let mut parts = Vec::with_capacity(rules.len());
            for r in rules {
                parts.push(expr_to_sql(&r.require.node, &sql_ctx)?);
            }
            Some(Some(format!("NOT ({})", parts.join(" OR "))))
        };

        let mut rules_sql = String::new();
        let mut trigger_warnings = Vec::new();
        for rule in &ordered {
            if rule.overrides.is_some() { continue; } // pure skip-switch, never fails on its own

            let translated = (|| -> Option<(String, String)> {
                let req_sql = expr_to_sql(&rule.require.node, &sql_ctx)?;
                let message = variant_message(&rule.else_.node)?;

                let mut conds: Vec<String> = Vec::new();
                for after in &rule.after {
                    conds.push(req_sql_by_name.get(after.node.as_str())?.clone());
                }
                match override_guard_sql(rule.name.node.as_str()) {
                    Some(Some(guard)) => conds.push(guard),
                    Some(None) => return None, // overriding rule's own condition didn't translate
                    None => {}
                }
                conds.push(format!("NOT ({})", req_sql));
                Some((conds.join(" AND "), message))
            })();

            match translated {
                Some((gate, message)) => {
                    writeln!(rules_sql, "    -- Rule: {}", rule.name.node).unwrap();
                    writeln!(rules_sql, "    IF {} THEN RAISE EXCEPTION '{}'; END IF;",
                        gate, message.replace('\'', "''")).unwrap();
                }
                None => {
                    writeln!(rules_sql, "    -- Rule: {}", rule.name.node).unwrap();
                    // Plain ASCII only (no em-dash etc.) — this text is emitted into
                    // real SQL piped through `psql`, which on at least this platform's
                    // pipeline does not reliably preserve non-ASCII bytes as UTF-8 (a
                    // real, reproduced failure: "invalid byte sequence for encoding
                    // UTF8" from a single em-dash character in this exact comment).
                    writeln!(rules_sql, "    -- (could not be translated to SQL - not enforced by this trigger)").unwrap();
                    trigger_warnings.push(format!(
                        "validator `{}`'s rule `{}` could not be translated to trigger SQL — it is NOT enforced by the installed database trigger (only enforced at the application layer via `{}.validate`/`validateAll`)",
                        vname, rule.name.node, vname));
                }
            }
        }
        out.trigger_warnings = trigger_warnings;

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

/// Build a name → parenthesized Certo source map for every module-level
/// `temporal` declaration (BACKLOG item 317(b)) — unlike `constraint`,
/// temporals don't compose (spec §16.9: "temporals are fully resolved at
/// declaration time"), so this is a direct render, no recursion or cycle
/// guard needed like `build_constraint_bodies`'s own `resolve_constraint`.
pub fn build_temporal_bodies(temporals: &HashMap<&str, &S<Expr>>) -> HashMap<String, String> {
    temporals.iter()
        .map(|(&name, body)| (name.to_string(), format!("({})", fmt_expr(&body.node, 0))))
        .collect()
}

/// Replace every standalone identifier in `text` that names a known
/// constraint (or, via the same generic substitution, a known `temporal`)
/// with that declaration's own expanded, parenthesized body. Skips content
/// inside string literals and identifiers immediately following `.` (a
/// field/qualified-path segment, never a bare reference).
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

/// Topological sort of rules respecting `after` dependencies. Rules with no
/// dependencies come first; rules with dependencies follow. BACKLOG item
/// 316 — `after` is still a hard ordering constraint (a prerequisite is
/// always visited, and thus ordered, before its dependent, regardless of
/// priority), but wherever `after` itself leaves the relative order among
/// several rules unconstrained, a higher `priority` (default 0, per the
/// spec's own §16.4 definition — "higher values are evaluated first") now
/// breaks the tie, rather than the previous accidental declaration order.
/// This makes `priority` a real, observable evaluation-order signal for
/// `validate`'s fail-fast chain (which of several otherwise-independent
/// failing rules is the one actually returned) and `validateAll`'s
/// error-list order — deliberately *not* a fix for resolving conflicts
/// among multiple rules that `overrides` the very same target rule, which
/// stays exactly as item 218 built it (an overriding rule never
/// independently fails, so there is no "winner" for priority to pick
/// between multiple simultaneously-active overriders — see that item's own
/// note, and this item's BACKLOG writeup, for why that's a separate, bigger
/// piece of design work, not bundled in here).
fn topo_sort_rules<'a>(rules: &'a [RuleDecl]) -> Vec<&'a RuleDecl> {
    let idx: HashMap<&str, usize> = rules.iter()
        .enumerate()
        .map(|(i, r)| (r.name.node.as_str(), i))
        .collect();
    let priority_of = |i: usize| rules[i].priority.unwrap_or(0);

    let mut visited = vec![false; rules.len()];
    let mut order: Vec<usize> = Vec::with_capacity(rules.len());

    fn visit(i: usize, rules: &[RuleDecl], idx: &HashMap<&str, usize>,
             priority_of: &dyn Fn(usize) -> i64,
             visited: &mut Vec<bool>, order: &mut Vec<usize>) {
        if visited[i] { return; }
        visited[i] = true;
        let mut deps: Vec<usize> = rules[i].after.iter()
            .filter_map(|after| idx.get(after.node.as_str()).copied())
            .collect();
        deps.sort_by_key(|&j| (std::cmp::Reverse(priority_of(j)), j));
        for j in deps {
            visit(j, rules, idx, priority_of, visited, order);
        }
        order.push(i);
    }

    let mut roots: Vec<usize> = (0..rules.len()).collect();
    roots.sort_by_key(|&i| (std::cmp::Reverse(priority_of(i)), i));
    for i in roots {
        visit(i, rules, &idx, &priority_of, &mut visited, &mut order);
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

// ------------------------------------------------------------------ //
// Trigger SQL translation (BACKLOG item 243)
// ------------------------------------------------------------------ //

/// A context field's `loaded by db.<table>.find(key)` resolved to a real,
/// correlated subquery — `table` is the raw table name, `key_sql` the
/// already-translated key expression (typically `NEW.some_column`).
struct ContextSubquery {
    table:   String,
    key_sql: String,
}

struct SqlCtx<'a> {
    entity_var:         &'a str,
    context_subqueries: &'a HashMap<&'a str, ContextSubquery>,
    constraints:        &'a HashMap<&'a str, &'a S<Expr>>,
    temporals:          &'a HashMap<&'a str, &'a S<Expr>>,
}

/// Recognizes a context field's `loaded by` expression as the one real,
/// structural convention item 226 already establishes for the ambient `db`
/// accessor sugar — `db.<table>.find(<key-expr>)` — returning the table
/// name and the (untranslated) key expression. Any other shape (an
/// arbitrary function call) can't be turned into a subquery here; the
/// caller falls back to leaving that context field's dependent rules
/// untranslated rather than guessing.
fn context_field_table_and_key(loaded_by: &Expr) -> Option<(&str, &Expr)> {
    let Expr::App { func, args, .. } = loaded_by else { return None };
    if args.len() != 1 { return None; }
    let Expr::Field { expr: mid, field: method, .. } = &func.node else { return None };
    if method.node != "find" { return None; }
    let Expr::Field { expr: inner, field: table, .. } = &mid.node else { return None };
    let Expr::Path { path, .. } = &inner.node else { return None };
    if path.segments.len() != 1 || path.segments[0].node != "db" { return None; }
    Some((table.node.as_str(), &args[0].value.node))
}

/// Extracts a `RAISE EXCEPTION` message from a rule's `else` clause — the
/// qualified error-variant reference every `else` clause already is,
/// `Err(...)`-wrapped by the *Certo*-source codegen paths above but never
/// written that way by the user (confirmed: `rule.else_` itself is always
/// the bare variant, e.g. `OE.X`). Confirmed by direct AST inspection: a
/// qualified constructor reference like `OrderError.NotDraft` parses as a
/// *single* `Expr::Path` with a 2-segment `ModulePath` (`["OrderError",
/// "NotDraft"]`) — NOT `Expr::Field` (that shape is for value-level field
/// access, e.g. `order.status`; this is a type-level/constructor reference,
/// parsed the same way `List.map` is). A payload-carrying variant
/// (`OE.X(v)`, `Expr::App` wrapping that same `Path`) still resolves to its
/// own qualified name — the payload itself can't meaningfully cross into a
/// raw SQL exception message, matching the user-confirmed "variant name as
/// the message" scope for this item.
fn variant_message(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path { path, .. } if path.segments.len() == 2 =>
            Some(format!("{}.{}", path.segments[0].node, path.segments[1].node)),
        Expr::App { func, .. } => variant_message(&func.node),
        _ => None,
    }
}

/// Constant-folds a `Duration.days(N)`/`.hours(N)`/`.minutes(N)`/
/// `.seconds(N)`/`.milliseconds(N)` call (or a `temporal` declaration
/// resolving to one) down to a millisecond count, matching the exact
/// multipliers `certo_duration_*` uses at runtime
/// (`crates/stdlib/src/datetime.rs`) — a `.age` comparison needs this
/// constant to render a SQL `interval` literal, since PL/pgSQL has no
/// first-class "Certo Duration" value to pass through directly.
fn resolve_duration_ms(expr: &Expr, ctx: &SqlCtx) -> Option<i64> {
    match expr {
        Expr::Path { path, .. } if path.segments.len() == 1 => {
            let body = ctx.temporals.get(path.segments[0].node.as_str())?;
            resolve_duration_ms(&body.node, ctx)
        }
        Expr::App { func, args, .. } => {
            let Expr::Field { expr: recv, field, .. } = &func.node else { return None };
            let Expr::Path { path, .. } = &recv.node else { return None };
            if path.segments.len() != 1 || path.segments[0].node != "Duration" { return None; }
            let [arg] = args.as_slice() else { return None };
            let Expr::Lit { value: Lit::Int(n), .. } = &arg.value.node else { return None };
            let mult: i64 = match field.node.as_str() {
                "milliseconds" => 1,
                "seconds"      => 1_000,
                "minutes"      => 60_000,
                "hours"        => 3_600_000,
                "days"         => 86_400_000,
                _ => return None,
            };
            Some(n * mult)
        }
        _ => None,
    }
}

/// True for `expr.age` (BACKLOG item 226/243's own `Expr::Age` postfix).
fn is_age(expr: &Expr) -> bool { matches!(expr, Expr::Age { .. }) }

/// Translates a `require`/override-guard/context-key boolean (or, for a
/// context-key expression, scalar) expression to a SQL fragment. Returns
/// `None` — not a hard error — for anything it doesn't structurally
/// recognize, so a single untranslatable rule degrades gracefully (falls
/// back to the pre-existing comment placeholder) without blocking the rest
/// of the trigger. Deliberately narrow: this only needs to cover the real
/// shapes validator rules actually use today, not general Certo expressions.
fn expr_to_sql(expr: &Expr, ctx: &SqlCtx) -> Option<String> {
    match expr {
        Expr::Lit { value, .. } => match value {
            Lit::Bool(b) => Some(if *b { "TRUE".to_string() } else { "FALSE".to_string() }),
            Lit::Int(n)  => Some(n.to_string()),
            Lit::String(s) => Some(format!("'{}'", s.replace('\'', "''"))),
            _ => None, // Float/Decimal/FString/Uuid/Unit: not needed by any real rule today
        },

        // A bare identifier is only ever translatable as a named
        // `constraint` reference — inlined recursively. An enum-variant
        // literal (`Draft`) is only ever meaningful as one *side* of a
        // comparison, handled by the `BinOp` arm below (matching item
        // 244's own `is_bare_literal` heuristic), not standalone here.
        Expr::Path { path, .. } if path.segments.len() == 1 => {
            let body = ctx.constraints.get(path.segments[0].node.as_str())?;
            expr_to_sql(&body.node, ctx)
        }

        Expr::Field { expr: inner, field, .. } => {
            let Expr::Path { path, .. } = &inner.node else { return None };
            if path.segments.len() != 1 { return None; }
            let name = path.segments[0].node.as_str();
            if name == ctx.entity_var {
                Some(format!("NEW.{}", snake_case(&field.node)))
            } else if let Some(sub) = ctx.context_subqueries.get(name) {
                Some(format!("(SELECT {} FROM {} WHERE id = {})", snake_case(&field.node), sub.table, sub.key_sql))
            } else {
                None
            }
        }

        Expr::UnOp { op: UnOp::Not, expr: inner, .. } =>
            Some(format!("NOT ({})", expr_to_sql(&inner.node, ctx)?)),
        Expr::UnOp { op: UnOp::Neg, .. } => None,

        Expr::BinOp { op: op @ (BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq),
                      left, right, .. } => {
            // `.age` comparison — special-cased as a whole, since there's
            // no standalone SQL value to produce for `.age` on its own.
            if is_age(&left.node) || is_age(&right.node) {
                let (age_expr, other, flipped) = if let Expr::Age { expr: inner, .. } = &left.node {
                    (inner, &right.node, false)
                } else if let Expr::Age { expr: inner, .. } = &right.node {
                    (inner, &left.node, true)
                } else { unreachable!() };
                let inner_sql = expr_to_sql(&inner_age_base(age_expr), ctx)?;
                let ms = resolve_duration_ms(other, ctx)?;
                let sql_op = match (op, flipped) {
                    (BinOp::Eq, _)         => "=",
                    (BinOp::NotEq, _)      => "!=",
                    (BinOp::Lt, false)     => "<",
                    (BinOp::Lt, true)      => ">",
                    (BinOp::LtEq, false)   => "<=",
                    (BinOp::LtEq, true)    => ">=",
                    (BinOp::Gt, false)     => ">",
                    (BinOp::Gt, true)      => "<",
                    (BinOp::GtEq, false)   => ">=",
                    (BinOp::GtEq, true)    => "<=",
                    _ => return None,
                };
                return Some(format!("(now() - {}) {} interval '{} milliseconds'", inner_sql, sql_op, ms));
            }

            // Bare enum-variant literal on either side (item 244's own
            // `is_bare_literal` heuristic, generalized) — quote it as a
            // real SQL string rather than trying to translate it as a
            // constraint/entity/context reference.
            let is_bare_variant = |e: &Expr| matches!(e, Expr::Path { path, .. } if path.segments.len() == 1
                && !ctx.constraints.contains_key(path.segments[0].node.as_str()));
            let side_sql = |e: &Expr| -> Option<String> {
                if let Expr::Path { path, .. } = e {
                    if is_bare_variant(e) { return Some(format!("'{}'", path.segments[0].node)); }
                }
                expr_to_sql(e, ctx)
            };

            let l = side_sql(&left.node)?;
            let r = side_sql(&right.node)?;
            let sql_op = match op {
                BinOp::Eq    => "=",
                BinOp::NotEq => "!=",
                BinOp::Lt    => "<",
                BinOp::LtEq  => "<=",
                BinOp::Gt    => ">",
                BinOp::GtEq  => ">=",
                _ => unreachable!(),
            };
            Some(format!("({} {} {})", l, sql_op, r))
        }

        Expr::BinOp { op: BinOp::And, left, right, .. } =>
            Some(format!("({} AND {})", expr_to_sql(&left.node, ctx)?, expr_to_sql(&right.node, ctx)?)),
        Expr::BinOp { op: BinOp::Or, left, right, .. } =>
            Some(format!("({} OR {})", expr_to_sql(&left.node, ctx)?, expr_to_sql(&right.node, ctx)?)),

        // `.isSome()` / `.isNone()` on a field.
        Expr::App { func, args, .. } if args.is_empty() => {
            let Expr::Field { expr: recv, field, .. } = &func.node else { return None };
            let recv_sql = expr_to_sql(&recv.node, ctx)?;
            match field.node.as_str() {
                "isSome" => Some(format!("{} IS NOT NULL", recv_sql)),
                "isNone" => Some(format!("{} IS NULL", recv_sql)),
                _ => None,
            }
        }

        _ => None,
    }
}

/// `Expr::Age { expr, .. }` wraps the base expression `Box<S<Expr>>` — this
/// just unwraps one layer so callers can hand `expr_to_sql` a plain `&Expr`.
fn inner_age_base(expr: &S<Expr>) -> &Expr { &expr.node }
