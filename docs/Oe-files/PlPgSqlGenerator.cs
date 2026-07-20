using System.Text;
using RuleGenerator.Models;

namespace RuleGenerator.Generators;

/// <summary>
/// Generates PL/pgSQL validator functions from business rule definitions.
///
/// Output structure:
///   plpgsql/
///     004_rule_types.sql        — error code type, exception helpers
///     005_validators/
///       {entity}_{trigger}.sql  — one function per entity+trigger group
///     006_rule_triggers.sql     — BEFORE INSERT/UPDATE triggers wiring validators
///     007_api_helpers.sql       — callable wrappers for application layer use
///
/// Design principles:
///   - One validator function per entity+trigger combination
///   - Each rule becomes a named sub-check within the function
///   - depends_on implemented via early-exit variable flags
///   - overrides implemented via a BOOLEAN skip flag per overridden rule
///   - Named constraints inlined at generation time
///   - Temporal conditions use interval arithmetic against NOW()
///   - Composite conditions emit nested AND / OR / NOT expressions
///   - Functions are idempotent via CREATE OR REPLACE
///   - All functions follow a consistent signature:
///       validate_{entity}_{trigger}(NEW record, context JSONB) RETURNS VOID
///   - RAISE EXCEPTION carries structured error: 'RULE_CODE|human message'
///     so application code can parse both parts
/// </summary>
public class PlPgSqlGenerator : IGenerator
{
    public string Name => "plpgsql";

    // Maps our primitive type names to Postgres types
    private static readonly Dictionary<string, string> PgTypeMap = new()
    {
        ["text"]      = "TEXT",
        ["integer"]   = "INTEGER",
        ["boolean"]   = "BOOLEAN",
        ["uuid"]      = "UUID",
        ["decimal"]   = "DECIMAL",
        ["timestamp"] = "TIMESTAMPTZ",
        ["date"]      = "DATE",
        ["time"]      = "TIME",
        ["jsonb"]     = "JSONB",
        ["bytea"]     = "BYTEA"
    };

    private static readonly Dictionary<string, string> SqlOperatorMap = new()
    {
        ["eq"]         = "=",
        ["not_eq"]     = "!=",
        ["gt"]         = ">",
        ["gte"]        = ">=",
        ["lt"]         = "<",
        ["lte"]        = "<=",
        ["exists"]     = "IS NOT NULL",
        ["not_exists"] = "IS NULL"
    };

    public async Task GenerateAsync(ResolvedDomain domain, string outputPath)
    {
        Directory.CreateDirectory(outputPath);
        var validatorsPath = Path.Combine(outputPath, "005_validators");
        Directory.CreateDirectory(validatorsPath);

        // 1. Error infrastructure
        await WriteFile(outputPath, "004_rule_types.sql", BuildRuleTypes());

        // 2. One validator function per entity+trigger
        var groups = domain.Rules
            .GroupBy(r => (r.Entity, r.Trigger))
            .ToDictionary(g => g.Key, g => g.ToList());

        foreach (var ((entity, trigger), rules) in groups.OrderBy(g => g.Key))
        {
            var filename = $"{ToSnakeCase(entity)}_{trigger}.sql";
            var content = BuildValidatorFunction(entity, trigger, rules, domain);
            await WriteFile(validatorsPath, filename, content);
            Console.WriteLine($"[plpgsql] 005_validators/{filename}");
        }

        // 3. Trigger wiring
        await WriteFile(outputPath, "006_rule_triggers.sql",
            BuildRuleTriggers(domain, groups));

        // 4. API helper wrappers
        await WriteFile(outputPath, "007_api_helpers.sql",
            BuildApiHelpers(domain, groups));

        Console.WriteLine($"[plpgsql] 004_rule_types.sql");
        Console.WriteLine($"[plpgsql] 006_rule_triggers.sql");
        Console.WriteLine($"[plpgsql] 007_api_helpers.sql");
    }

    // =========================================================================
    // RULE TYPES — shared infrastructure
    // =========================================================================

    private static string BuildRuleTypes()
    {
        var sb = new StringBuilder();
        SqlHeader(sb, "Rule Validation Infrastructure");
        sb.AppendLine("-- Shared types and helpers used by all generated validator functions.");
        sb.AppendLine();

        // Structured exception raiser
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("-- raise_rule_violation(code, message)");
        sb.AppendLine("-- Raises a structured exception parseable by application code.");
        sb.AppendLine("-- Format: RULE_CODE|Human readable message");
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("CREATE OR REPLACE FUNCTION raise_rule_violation(");
        sb.AppendLine("    p_code    TEXT,");
        sb.AppendLine("    p_message TEXT");
        sb.AppendLine(") RETURNS VOID AS $$");
        sb.AppendLine("BEGIN");
        sb.AppendLine("    RAISE EXCEPTION USING");
        sb.AppendLine("        ERRCODE = 'P0001',");
        sb.AppendLine("        MESSAGE = p_code || '|' || p_message,");
        sb.AppendLine("        HINT    = p_message;");
        sb.AppendLine("END;");
        sb.AppendLine("$$ LANGUAGE plpgsql;");
        sb.AppendLine();

        // Context builder helper — assembles JSONB context from current_setting
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("-- current_rule_context()");
        sb.AppendLine("-- Returns context JSONB set by the application before calling validators.");
        sb.AppendLine("-- Application sets: SET LOCAL rule.user_id = '...';");
        sb.AppendLine("--                   SET LOCAL rule.user_role = 'ADMIN';");
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("CREATE OR REPLACE FUNCTION current_rule_context()");
        sb.AppendLine("RETURNS JSONB AS $$");
        sb.AppendLine("BEGIN");
        sb.AppendLine("    RETURN jsonb_build_object(");
        sb.AppendLine("        'user_id',   current_setting('rule.user_id',   TRUE),");
        sb.AppendLine("        'user_role', current_setting('rule.user_role', TRUE)");
        sb.AppendLine("    );");
        sb.AppendLine("END;");
        sb.AppendLine("$$ LANGUAGE plpgsql STABLE;");
        sb.AppendLine();

        // Age helper
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("-- record_age_days(ts TIMESTAMPTZ) → NUMERIC");
        sb.AppendLine("-- Returns how many days ago a timestamp occurred.");
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("CREATE OR REPLACE FUNCTION record_age_days(ts TIMESTAMPTZ)");
        sb.AppendLine("RETURNS NUMERIC AS $$");
        sb.AppendLine("BEGIN");
        sb.AppendLine("    RETURN EXTRACT(EPOCH FROM (NOW() - ts)) / 86400.0;");
        sb.AppendLine("END;");
        sb.AppendLine("$$ LANGUAGE plpgsql STABLE;");

        return sb.ToString();
    }

    // =========================================================================
    // VALIDATOR FUNCTION — one per entity+trigger
    // =========================================================================

    private string BuildValidatorFunction(
        string entity,
        string trigger,
        List<RuleDefinition> rules,
        ResolvedDomain domain)
    {
        var sb = new StringBuilder();
        var funcName = $"validate_{ToSnakeCase(entity)}_{trigger}";
        var tableName = domain.FindEntity(entity)?.TableName ?? ToSnakeCase(entity) + "s";

        SqlHeader(sb, $"Validator: {entity}.{trigger}");

        // Rule index
        sb.AppendLine($"-- Rules enforced ({rules.Count}):");
        foreach (var rule in rules)
            sb.AppendLine($"--   {rule.Id}: {rule.Description}");
        sb.AppendLine();

        // Function signature
        // NEW record passed as JSONB for flexibility across triggers and direct calls
        sb.AppendLine($"CREATE OR REPLACE FUNCTION {funcName}(");
        sb.AppendLine($"    p_record  JSONB,     -- NEW row as JSONB");
        sb.AppendLine($"    p_context JSONB       -- rule context (user_id, user_role, etc.)");
        sb.AppendLine($") RETURNS VOID AS $$");
        sb.AppendLine("DECLARE");

        // Declare local variables for depends_on flags and overrideable rule results
        var ordered = OrderRules(rules);
        var overrideMap = BuildOverrideMap(ordered);

        // Variable declarations
        DeclareVariables(sb, ordered, overrideMap, domain);

        sb.AppendLine("BEGIN");
        sb.AppendLine();

        // Extract commonly used context fields
        sb.AppendLine("    -- Extract context");
        sb.AppendLine("    v_user_role := p_context->>'user_role';");
        sb.AppendLine();

        // Emit rule evaluations
        EmitRuleEvaluations(sb, ordered, overrideMap, domain);

        sb.AppendLine("END;");
        sb.AppendLine($"$$ LANGUAGE plpgsql;");
        sb.AppendLine();

        // Add a convenience overload that reads context from session settings
        sb.AppendLine($"-- Convenience wrapper — reads context from current session settings.");
        sb.AppendLine($"CREATE OR REPLACE FUNCTION {funcName}(p_record JSONB)");
        sb.AppendLine($"RETURNS VOID AS $$");
        sb.AppendLine($"BEGIN");
        sb.AppendLine($"    PERFORM {funcName}(p_record, current_rule_context());");
        sb.AppendLine($"END;");
        sb.AppendLine($"$$ LANGUAGE plpgsql;");

        return sb.ToString();
    }

    private void DeclareVariables(
        StringBuilder sb,
        List<RuleDefinition> ordered,
        Dictionary<string, string> overrideMap,
        ResolvedDomain domain)
    {
        sb.AppendLine("    v_user_role TEXT;");

        // One BOOLEAN per depends_on rule to track pass/fail
        var dependedOn = ordered
            .SelectMany(r => r.DependsOn)
            .Distinct()
            .ToHashSet();

        foreach (var ruleId in dependedOn)
            sb.AppendLine($"    v_{ToVarName(ruleId)}_passed BOOLEAN := FALSE;");

        // One BOOLEAN per override rule to track if override condition passed
        foreach (var rule in ordered.Where(r => r.Overrides != null))
            sb.AppendLine($"    v_{ToVarName(rule.Id)}_active BOOLEAN := FALSE;");

        sb.AppendLine();
    }

    private void EmitRuleEvaluations(
        StringBuilder sb,
        List<RuleDefinition> ordered,
        Dictionary<string, string> overrideMap,
        ResolvedDomain domain)
    {
        foreach (var rule in ordered)
        {
            var varName = ToVarName(rule.Id);
            var isDepended = ordered.Any(r => r.DependsOn.Contains(rule.Id));
            var isOverrideRule = rule.Overrides != null;
            var isOverridden = overrideMap.ContainsKey(rule.Id);

            sb.AppendLine($"    -- {rule.Description}");

            // Override rules: evaluate condition and set flag, don't throw
            if (isOverrideRule)
            {
                sb.AppendLine($"    -- (override rule — suspends '{rule.Overrides}' when condition passes)");
                var cond = EmitCondition(rule.Condition, domain, "p_record", "p_context");
                sb.AppendLine($"    IF {cond} THEN");
                sb.AppendLine($"        v_{ToVarName(rule.Id)}_active := TRUE;");
                sb.AppendLine($"    END IF;");
                sb.AppendLine();
                continue;
            }

            // Build the evaluation block
            var lines = new List<string>();

            // depends_on: only proceed if prerequisite passed
            if (rule.DependsOn.Any())
            {
                var depChecks = string.Join(" AND ",
                    rule.DependsOn.Select(d => $"v_{ToVarName(d)}_passed"));
                lines.Add($"IF {depChecks} THEN");
            }

            // Overridden: skip if overriding rule condition was true
            string innerIndent = rule.DependsOn.Any() ? "        " : "    ";
            if (isOverridden)
            {
                var overridingId = overrideMap[rule.Id];
                lines.Add($"{innerIndent}IF NOT v_{ToVarName(overridingId)}_active THEN");
                innerIndent += "    ";
            }

            // The actual condition check
            var condExpr = EmitCondition(rule.Condition, domain, "p_record", "p_context");
            lines.Add($"{innerIndent}IF NOT ({condExpr}) THEN");
            lines.Add($"{innerIndent}    PERFORM raise_rule_violation(");
            lines.Add($"{innerIndent}        '{rule.Error.Code}',");
            lines.Add($"{innerIndent}        '{EscapeSql(rule.Error.Message)}');");
            lines.Add($"{innerIndent}END IF;");

            // Track pass/fail for depends_on
            if (isDepended)
            {
                lines.Add($"{innerIndent}v_{varName}_passed := ({condExpr});");
            }

            // Close override block
            if (isOverridden)
            {
                innerIndent = innerIndent[4..];
                lines.Add($"{innerIndent}END IF;");
            }

            // Close depends_on block
            if (rule.DependsOn.Any())
                lines.Add($"END IF;");

            foreach (var line in lines)
                sb.AppendLine(line.StartsWith("    ") ? line : $"    {line}");

            sb.AppendLine();
        }
    }

    // =========================================================================
    // CONDITION EMISSION
    // =========================================================================

    private string EmitCondition(
        ConditionNode node,
        ResolvedDomain domain,
        string record,
        string ctx)
    {
        // Composite: all → AND, any → OR, none → NOT (OR)
        if (node.All != null)
            return WrapComposite(node.All.Select(c =>
                EmitCondition(c, domain, record, ctx)), "AND");

        if (node.Any != null)
            return WrapComposite(node.Any.Select(c =>
                EmitCondition(c, domain, record, ctx)), "OR");

        if (node.None != null)
            return "NOT (" + WrapComposite(node.None.Select(c =>
                EmitCondition(c, domain, record, ctx)), "OR") + ")";

        // Named constraint — inline condition
        if (node.IsConstraintRef)
            return $"TRUE /* constraint '{node.Constraint}' — inlined by resolver */";

        // Named temporal — emit interval arithmetic
        if (node.IsTemporalRef)
            return EmitTemporalCondition(node);

        // Field comparison leaf
        if (node.IsFieldComparison)
            return EmitFieldComparison(node, record, ctx);

        return "TRUE -- unresolved condition";
    }

    private static string WrapComposite(IEnumerable<string> parts, string op)
    {
        var list = parts.ToList();
        if (!list.Any()) return "TRUE";
        if (list.Count == 1) return list[0];
        return "(\n        " +
               string.Join($"\n        {op} ", list.Select(p => $"({p})")) +
               "\n    )";
    }

    private static string EmitFieldComparison(ConditionNode node, string record, string ctx)
    {
        var field = node.Field!;
        var op = node.Operator!;
        var value = node.Value;

        // Determine if value is a field reference or literal
        var isFieldRef = value is string s && s.Contains('.');

        // Resolve left-hand side
        var lhs = FieldToSql(field, record, ctx);

        // Handle operators that don't use a RHS
        if (op == "exists")    return $"{lhs} IS NOT NULL";
        if (op == "not_exists") return $"{lhs} IS NULL";

        // Resolve right-hand side
        var rhs = isFieldRef
            ? FieldToSql(value!.ToString()!, record, ctx)
            : FormatSqlLiteral(value);

        return op switch
        {
            "eq"          => $"{lhs} = {rhs}",
            "not_eq"      => $"{lhs} != {rhs}",
            "gt"          => $"{lhs} > {rhs}",
            "gte"         => $"{lhs} >= {rhs}",
            "lt"          => $"{lhs} < {rhs}",
            "lte"         => $"{lhs} <= {rhs}",
            "in"          => EmitInSql(lhs, value, negate: false),
            "not_in"      => EmitInSql(lhs, value, negate: true),
            "matches"     => $"{lhs} ~ '{EscapeSql(value?.ToString() ?? "")}'",
            "not_matches" => $"{lhs} !~ '{EscapeSql(value?.ToString() ?? "")}'",
            "contains"    => $"{lhs} LIKE '%' || {rhs} || '%'",
            "starts_with" => $"{lhs} LIKE {rhs} || '%'",
            _             => $"TRUE -- unknown operator: {op}"
        };
    }

    private static string EmitTemporalCondition(ConditionNode node)
    {
        // temporal id and duration/unit are resolved by the resolver
        // here we emit the pattern; the resolver inlines the actual numbers
        var field = node.Field ?? "created_at";
        var op = node.Operator ?? "age_lt";
        var temporal = node.Temporal ?? "unknown";

        // Format: record_age_days(p_record->>'created_at'::TIMESTAMPTZ) < 30
        var fieldSql = $"(p_record->>'{ToSnakeCase(field)}')::TIMESTAMPTZ";
        var ageSql = $"record_age_days({fieldSql})";

        return op switch
        {
            "age_lt"  => $"{ageSql} < /* {temporal}.days */",
            "age_gt"  => $"{ageSql} > /* {temporal}.days */",
            "age_lte" => $"{ageSql} <= /* {temporal}.days */",
            "age_gte" => $"{ageSql} >= /* {temporal}.days */",
            _         => $"TRUE -- unknown temporal operator: {op}"
        };
    }

    private static string EmitInSql(string lhs, object? value, bool negate)
    {
        string values;
        if (value is List<object> items)
            values = string.Join(", ", items.Select(FormatSqlLiteral));
        else
            values = FormatSqlLiteral(value);

        var expr = $"{lhs} IN ({values})";
        return negate ? $"{lhs} NOT IN ({values})" : expr;
    }

    // =========================================================================
    // TRIGGERS
    // =========================================================================

    private string BuildRuleTriggers(
        ResolvedDomain domain,
        Dictionary<(string, string), List<RuleDefinition>> groups)
    {
        var sb = new StringBuilder();
        SqlHeader(sb, "Rule Enforcement Triggers");
        sb.AppendLine("-- BEFORE INSERT/UPDATE triggers that call validator functions.");
        sb.AppendLine("-- Validators only fire for triggers that have rules defined.");
        sb.AppendLine();

        // Map trigger verbs to SQL operations
        var sqlOpMap = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
        {
            ["create"] = "INSERT",
            ["update"] = "UPDATE",
            ["submit"] = "UPDATE",
            ["approve"] = "UPDATE",
            ["cancel"] = "UPDATE",
            ["void"] = "UPDATE",
            ["issue"] = "UPDATE",
            ["pay"] = "UPDATE",
            ["dispatch"] = "UPDATE",
            ["return"] = "UPDATE",
            ["apply_discount"] = "UPDATE",
            ["set_priority"] = "UPDATE"
        };

        // Group by entity, then map triggers to SQL events
        var byEntity = groups
            .GroupBy(g => g.Key.Item1)
            .OrderBy(g => g.Key);

        foreach (var entityGroup in byEntity)
        {
            var entity = entityGroup.Key;
            var resolvedEntity = domain.FindEntity(entity);
            if (resolvedEntity == null) continue;

            var tableName = resolvedEntity.TableName;
            var triggers = entityGroup
                .Select(g => (trigger: g.Key.Item2,
                              sqlOp: sqlOpMap.GetValueOrDefault(g.Key.Item2, "UPDATE")))
                .ToList();

            // Consolidate: one trigger function per entity covering all operations
            var funcName = $"trg_validate_{tableName}";

            sb.AppendLine($"-- {entity} — {triggers.Count} trigger(s): " +
                          string.Join(", ", triggers.Select(t => t.trigger)));

            // Trigger function
            sb.AppendLine($"CREATE OR REPLACE FUNCTION {funcName}()");
            sb.AppendLine("RETURNS TRIGGER AS $$");
            sb.AppendLine("DECLARE");
            sb.AppendLine("    v_record  JSONB;");
            sb.AppendLine("    v_context JSONB;");
            sb.AppendLine("BEGIN");
            sb.AppendLine("    v_record  := row_to_json(NEW)::JSONB;");
            sb.AppendLine("    v_context := current_rule_context();");
            sb.AppendLine();
            sb.AppendLine("    IF TG_OP = 'INSERT' THEN");

            var insertTriggers = triggers.Where(t => t.sqlOp == "INSERT").ToList();
            if (insertTriggers.Any())
            {
                foreach (var (trigger, _) in insertTriggers)
                    sb.AppendLine($"        PERFORM validate_{tableName}_{trigger}" +
                                  "(v_record, v_context);");
            }
            else
            {
                sb.AppendLine("        -- no insert rules defined");
            }

            sb.AppendLine("    ELSIF TG_OP = 'UPDATE' THEN");

            // For updates, emit status-based routing using OLD.status
            var updateTriggers = triggers.Where(t => t.sqlOp == "UPDATE").ToList();
            if (updateTriggers.Any())
            {
                // Detect status-field-based triggers vs generic updates
                var hasStatusField = resolvedEntity.Fields
                    .Any(f => f.Kind == FieldKind.Enum && f.EnumType?.Transitions != null);

                if (hasStatusField)
                {
                    sb.AppendLine("        -- Route by status transition");
                    sb.AppendLine("        IF OLD.status IS DISTINCT FROM NEW.status THEN");
                    sb.AppendLine("            CASE NEW.status");

                    foreach (var (trigger, _) in updateTriggers)
                    {
                        // Map trigger name to likely target status
                        var targetStatus = TriggerToStatus(trigger);
                        if (targetStatus != null)
                        {
                            sb.AppendLine($"                WHEN '{targetStatus}' THEN");
                            sb.AppendLine($"                    PERFORM validate_{tableName}_{trigger}" +
                                          "(v_record, v_context);");
                        }
                    }

                    sb.AppendLine("                ELSE NULL; -- no validator for this transition");
                    sb.AppendLine("            END CASE;");
                    sb.AppendLine("        ELSE");
                    // Non-status updates — call generic update validators if any
                    var genericUpdates = updateTriggers
                        .Where(t => t.trigger is "update" or "apply_discount" or "set_priority")
                        .ToList();
                    foreach (var (trigger, _) in genericUpdates)
                        sb.AppendLine($"            PERFORM validate_{tableName}_{trigger}" +
                                      "(v_record, v_context);");
                    if (!genericUpdates.Any())
                        sb.AppendLine("            NULL; -- no non-status update rules");
                    sb.AppendLine("        END IF;");
                }
                else
                {
                    foreach (var (trigger, _) in updateTriggers)
                        sb.AppendLine($"        PERFORM validate_{tableName}_{trigger}" +
                                      "(v_record, v_context);");
                }
            }
            else
            {
                sb.AppendLine("        -- no update rules defined");
            }

            sb.AppendLine("    END IF;");
            sb.AppendLine();
            sb.AppendLine("    RETURN NEW;");
            sb.AppendLine("END;");
            sb.AppendLine("$$ LANGUAGE plpgsql;");
            sb.AppendLine();

            // Attach trigger to table
            sb.AppendLine($"DROP TRIGGER IF EXISTS {funcName} ON {tableName};");
            sb.AppendLine($"CREATE TRIGGER {funcName}");
            sb.AppendLine($"    BEFORE INSERT OR UPDATE ON {tableName}");
            sb.AppendLine($"    FOR EACH ROW");
            sb.AppendLine($"    EXECUTE FUNCTION {funcName}();");
            sb.AppendLine();
            sb.AppendLine(new string('-', 79));
            sb.AppendLine();
        }

        return sb.ToString();
    }

    // =========================================================================
    // API HELPERS
    // =========================================================================

    private string BuildApiHelpers(
        ResolvedDomain domain,
        Dictionary<(string, string), List<RuleDefinition>> groups)
    {
        var sb = new StringBuilder();
        SqlHeader(sb, "API Helper Functions");
        sb.AppendLine("-- Callable from application code to validate before committing.");
        sb.AppendLine("-- Returns JSONB with { valid, violations[] } rather than raising.");
        sb.AppendLine("-- Use these for pre-flight checks in REST handlers.");
        sb.AppendLine();

        // Generic safe-call wrapper
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("-- validate_safe(validator_func TEXT, record JSONB, context JSONB)");
        sb.AppendLine("-- Calls a validator function by name, catches violations,");
        sb.AppendLine("-- returns structured result rather than raising.");
        sb.AppendLine("-- ---------------------------------------------------------------------------");
        sb.AppendLine("CREATE OR REPLACE FUNCTION validate_safe(");
        sb.AppendLine("    p_func    TEXT,");
        sb.AppendLine("    p_record  JSONB,");
        sb.AppendLine("    p_context JSONB");
        sb.AppendLine(") RETURNS JSONB AS $$");
        sb.AppendLine("DECLARE");
        sb.AppendLine("    v_result     JSONB;");
        sb.AppendLine("    v_violations JSONB[] := '{}';");
        sb.AppendLine("    v_parts      TEXT[];");
        sb.AppendLine("BEGIN");
        sb.AppendLine("    EXECUTE format('SELECT %I($1, $2)', p_func)");
        sb.AppendLine("        USING p_record, p_context;");
        sb.AppendLine("    RETURN jsonb_build_object('valid', TRUE, 'violations', '[]'::JSONB);");
        sb.AppendLine("EXCEPTION WHEN OTHERS THEN");
        sb.AppendLine("    v_parts := string_to_array(SQLERRM, '|');");
        sb.AppendLine("    RETURN jsonb_build_object(");
        sb.AppendLine("        'valid', FALSE,");
        sb.AppendLine("        'violations', jsonb_build_array(jsonb_build_object(");
        sb.AppendLine("            'code',    v_parts[1],");
        sb.AppendLine("            'message', COALESCE(v_parts[2], SQLERRM)");
        sb.AppendLine("        ))");
        sb.AppendLine("    );");
        sb.AppendLine("END;");
        sb.AppendLine("$$ LANGUAGE plpgsql;");
        sb.AppendLine();

        // Per entity+trigger safe wrappers
        foreach (var ((entity, trigger), rules) in groups.OrderBy(g => g.Key))
        {
            var tableName = domain.FindEntity(entity)?.TableName
                            ?? ToSnakeCase(entity) + "s";
            var validatorFunc = $"validate_{tableName}_{trigger}";
            var helperFunc = $"preflight_{tableName}_{trigger}";

            sb.AppendLine($"-- Pre-flight check for {entity}.{trigger}");
            sb.AppendLine($"-- Returns: {{ \"valid\": bool, \"violations\": [{{\"code\", \"message\"}}] }}");
            sb.AppendLine($"CREATE OR REPLACE FUNCTION {helperFunc}(");
            sb.AppendLine($"    p_record  JSONB,");
            sb.AppendLine($"    p_context JSONB DEFAULT current_rule_context()");
            sb.AppendLine($") RETURNS JSONB AS $$");
            sb.AppendLine($"BEGIN");
            sb.AppendLine($"    RETURN validate_safe('{validatorFunc}', p_record, p_context);");
            sb.AppendLine($"END;");
            sb.AppendLine($"$$ LANGUAGE plpgsql;");
            sb.AppendLine();
        }

        return sb.ToString();
    }

    // =========================================================================
    // UTILITIES
    // =========================================================================

    /// <summary>
    /// Resolves a dot-notation field path to a SQL expression.
    /// "user.verified"   → p_context->>'user_verified' (context field)
    /// "order.total"     → (p_record->>'total')::DECIMAL (record field)
    /// "user.role"       → p_context->>'user_role'
    /// </summary>
    private static string FieldToSql(string path, string record, string ctx)
    {
        var parts = path.Split('.');

        if (parts.Length == 1)
        {
            // Bare field — comes from the record
            return $"({record}->>'{ToSnakeCase(parts[0])}')";
        }

        // Two parts: entity.field
        var entity = parts[0].ToLower();
        var field = ToSnakeCase(parts[1]);

        // User context fields come from context JSONB
        if (entity == "user")
            return $"({ctx}->>'user_{field}')";

        // Same-entity fields and cross-entity references both live in record
        // (cross-entity fields must be joined in before calling the validator)
        return $"({record}->>'{field}')";
    }

    private static string FormatSqlLiteral(object? value)
    {
        return value switch
        {
            null    => "NULL",
            bool b  => b ? "TRUE" : "FALSE",
            int i   => i.ToString(),
            long l  => l.ToString(),
            double d => d.ToString("G"),
            string s => $"'{EscapeSql(s)}'",
            List<object> list => string.Join(", ", list.Select(FormatSqlLiteral)),
            _ => $"'{EscapeSql(value.ToString() ?? "")}'"
        };
    }

    private static string? TriggerToStatus(string trigger) => trigger switch
    {
        "submit"   => "SUBMITTED",
        "approve"  => "APPROVED",
        "cancel"   => "CANCELLED",
        "dispatch" => "DISPATCHED",
        "void"     => "VOIDED",
        "issue"    => "ISSUED",
        "pay"      => "PAID",
        "return"   => "RETURNED",
        _          => null
    };

    private static Dictionary<string, string> BuildOverrideMap(List<RuleDefinition> rules) =>
        rules
            .Where(r => r.Overrides != null)
            .ToDictionary(r => r.Overrides!, r => r.Id);

    private static List<RuleDefinition> OrderRules(List<RuleDefinition> rules)
    {
        var sorted = rules.OrderByDescending(r => r.Priority).ToList();
        var result = new List<RuleDefinition>();
        var visited = new HashSet<string>();
        var byId = sorted.ToDictionary(r => r.Id);

        void Visit(RuleDefinition rule)
        {
            if (visited.Contains(rule.Id)) return;
            foreach (var dep in rule.DependsOn)
                if (byId.TryGetValue(dep, out var depRule)) Visit(depRule);
            visited.Add(rule.Id);
            result.Add(rule);
        }

        foreach (var rule in sorted) Visit(rule);
        return result;
    }

    private static void SqlHeader(StringBuilder sb, string title)
    {
        sb.AppendLine($"-- {'=',77}".Replace("=", new string('=', 77)));
        sb.AppendLine($"-- {title}");
        sb.AppendLine($"-- Generated: {DateTime.UtcNow:yyyy-MM-dd HH:mm:ss} UTC");
        sb.AppendLine($"-- DO NOT EDIT — regenerate from YAML definitions");
        sb.AppendLine($"-- {'=',77}".Replace("=", new string('=', 77)));
        sb.AppendLine();
    }

    private static string ToSnakeCase(string s)
    {
        var sb = new StringBuilder();
        for (int i = 0; i < s.Length; i++)
        {
            if (i > 0 && char.IsUpper(s[i])) sb.Append('_');
            sb.Append(char.ToLower(s[i]));
        }
        return sb.ToString();
    }

    private static string ToVarName(string ruleId) =>
        ruleId.Replace('-', '_');

    private static string EscapeSql(string s) =>
        s.Replace("'", "''");

    private static async Task WriteFile(string dir, string filename, string content) =>
        await File.WriteAllTextAsync(Path.Combine(dir, filename), content);
}
