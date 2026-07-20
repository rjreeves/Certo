use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use certo_ast::decl::{ValidatorDecl, RuleDecl, TriggerOp};
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
pub fn emit_validator(v: &ValidatorDecl) -> ValidatorOutput {
    let mut out = ValidatorOutput::default();

    let vname       = &v.name.node;
    let entity_ty   = fmt_type(&v.entity.node, 0);
    let errors_ty   = fmt_type(&v.errors.node, 0);
    let ctx_type    = format!("{}Context", vname);

    // Entity variable: lowercase first letter of entity type name.
    let entity_var  = lowercase_first(&entity_ty);

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
            let req = fmt_expr(&rule.require.node, 0);
            let err = fmt_expr(&rule.else_.node, 0);
            expr = format!("if !({}) then Err({}) else {}", req, err, expr);
        }
        out.validate_fn = format!("{} =\n    {}\n", sig, expr);
    }

    // ---------------------------------------------------------------- //
    // validateAll — collect all errors (list concatenation)
    // ---------------------------------------------------------------- //
    {
        let sig = build_fn_sig(vname, "validateAll", &entity_var, &entity_ty,
            &ctx_type, !v.context.is_empty(),
            &format!("List<{}>", errors_ty));

        // Each rule contributes `[err]` when it fails (and its `after`
        // prerequisites' conditions hold), or `[]` otherwise; concatenate all.
        let req_by_name: HashMap<&str, String> = ordered.iter()
            .map(|r| (r.name.node.as_str(), fmt_expr(&r.require.node, 0)))
            .collect();

        let mut parts: Vec<String> = Vec::new();
        for rule in &ordered {
            let req = fmt_expr(&rule.require.node, 0);
            let err = fmt_expr(&rule.else_.node, 0);
            let gates: Vec<String> = rule.after.iter()
                .filter_map(|a| req_by_name.get(a.node.as_str()))
                .map(|r| format!("({})", r))
                .collect();
            let cond = if gates.is_empty() {
                format!("!({})", req)
            } else {
                format!("{} && !({})", gates.join(" && "), req)
            };
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
        out.validate_all_fn = format!("{} =\n    {}\n", sig, body);
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
    if has_ctx {
        format!("fn {}_{}({}: {}, context: {}): {}",
            vname, method, entity_var, entity_ty, ctx_type, ret)
    } else {
        format!("fn {}_{}({}: {}): {}",
            vname, method, entity_var, entity_ty, ret)
    }
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
