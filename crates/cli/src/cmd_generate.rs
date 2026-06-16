//! `certo generate validators` — YAML → Certo source + PL/pgSQL generator.
//!
//! Reads three YAML definition files and emits:
//!   Certo source (always):
//!     - `constraints.cto`         (prepared-items constraints)
//!     - `temporals.cto`           (prepared-items temporals)
//!     - `<entity>-validators.cto` (one per entity, one validator per trigger)
//!
//!   PL/pgSQL (with --emit-sql <dir>):
//!     - `004_rule_types.sql`           (shared infrastructure)
//!     - `005_validators/<e>_<t>.sql`   (one per entity+trigger)
//!     - `006_rule_triggers.sql`        (trigger wiring)
//!     - `007_api_helpers.sql`          (preflight JSONB wrappers)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process;
use serde::Deserialize;

// ================================================================== //
// YAML data structures
// ================================================================== //

#[derive(Debug, Deserialize)]
struct PreparedItems {
    #[serde(default)]
    temporal:    Vec<TemporalDef>,
    #[serde(default)]
    constraints: Vec<ConstraintDef>,
    #[serde(default)]
    enums:       Vec<EnumDef>,
}

#[derive(Debug, Deserialize)]
struct TemporalDef {
    id:       String,
    duration: u64,
    unit:     String,
}

#[derive(Debug, Deserialize)]
struct ConstraintDef {
    id:        String,
    condition: ConditionNode,
}

#[derive(Debug, Deserialize)]
struct EnumDef {
    id:     String,
    #[allow(dead_code)]
    values: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Entities {
    #[serde(default)]
    entities: Vec<EntityDef>,
}

#[derive(Debug, Deserialize)]
struct EntityDef {
    id:     String,
    #[serde(default)]
    fields: Vec<FieldDef>,
}

#[derive(Debug, Deserialize)]
struct FieldDef {
    #[allow(dead_code)]
    name:   String,
    #[serde(rename = "type")]
    #[allow(dead_code)]
    ty:     String,
    #[serde(default)]
    #[allow(dead_code)]
    entity: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Rules {
    #[serde(default)]
    rules: Vec<RuleDef>,
}

#[derive(Debug, Deserialize)]
struct RuleDef {
    id:          String,
    description: String,
    entity:      String,
    trigger:     String,
    #[serde(default)]
    priority:    Option<i64>,
    #[serde(default)]
    depends_on:  Vec<String>,
    #[serde(default)]
    overrides:   Option<String>,
    condition:   ConditionNode,
    error:       ErrorDef,
}

#[derive(Debug, Deserialize)]
struct ErrorDef {
    code:    String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ConditionNode {
    ConstraintRef { constraint: String },
    TemporalRef   { temporal: String, field: String, operator: String },
    All           { all:  Vec<ConditionNode> },
    Any           { any:  Vec<ConditionNode> },
    None          { none: Vec<ConditionNode> },
    Leaf {
        field:    String,
        operator: String,
        #[serde(default)]
        value:    Option<serde_yaml::Value>,
        #[serde(default)]
        #[allow(dead_code)]
        unit:     Option<String>,
    },
}

// ================================================================== //
// Public entry point
// ================================================================== //

pub fn cmd_generate(args: &[String]) {
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_generate_help();
        return;
    }
    match args[0].as_str() {
        "validators" => cmd_generate_validators(&args[1..]),
        other => {
            eprintln!("Unknown generate subcommand: {}", other);
            eprintln!("Run `certo generate --help` for usage.");
            process::exit(2);
        }
    }
}

fn print_generate_help() {
    println!("Usage: certo generate <subcommand> [options]");
    println!();
    println!("Subcommands:");
    println!("  validators   Generate Certo validator source (and optionally PL/pgSQL) from YAML");
    println!();
    println!("Run `certo generate validators --help` for details.");
}

fn cmd_generate_validators(args: &[String]) {
    let mut prepared_items_path: Option<PathBuf> = None;
    let mut entities_path:       Option<PathBuf> = None;
    let mut rules_path:          Option<PathBuf> = None;
    let mut output_dir:          Option<PathBuf> = None;
    let mut sql_dir:             Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--prepared-items" => { i += 1; prepared_items_path = Some(next_path(args, i, "--prepared-items")); }
            "--entities"       => { i += 1; entities_path       = Some(next_path(args, i, "--entities")); }
            "--rules"          => { i += 1; rules_path          = Some(next_path(args, i, "--rules")); }
            "--output" | "-o"  => { i += 1; output_dir          = Some(next_path(args, i, "--output")); }
            "--emit-sql"       => { i += 1; sql_dir             = Some(next_path(args, i, "--emit-sql")); }
            "--help" | "-h" => {
                println!("Usage: certo generate validators \\");
                println!("    --prepared-items path/to/prepared-items.yaml \\");
                println!("    --entities       path/to/entities.yaml \\");
                println!("    --rules          path/to/rules.yaml \\");
                println!("    --output         src/validators/ \\");
                println!("    [--emit-sql      plpgsql/]");
                println!();
                println!("Certo output (--output):");
                println!("  constraints.cto         — named constraint declarations");
                println!("  temporals.cto           — named temporal window declarations");
                println!("  <entity>-validators.cto — one file per entity");
                println!();
                println!("PL/pgSQL output (--emit-sql):");
                println!("  004_rule_types.sql              — shared infrastructure");
                println!("  005_validators/<e>_<t>.sql      — one per entity+trigger");
                println!("  006_rule_triggers.sql           — BEFORE INSERT/UPDATE trigger wiring");
                println!("  007_api_helpers.sql             — preflight JSONB wrappers");
                return;
            }
            other if other.starts_with('-') => { eprintln!("Unknown option: {}", other); process::exit(2); }
            _ => { eprintln!("Unexpected argument: {}", args[i]); process::exit(2); }
        }
        i += 1;
    }

    let pi_path = prepared_items_path.unwrap_or_else(|| die("--prepared-items is required", 2));
    let en_path = entities_path      .unwrap_or_else(|| die("--entities is required", 2));
    let ru_path = rules_path         .unwrap_or_else(|| die("--rules is required", 2));
    let out_dir = output_dir         .unwrap_or_else(|| PathBuf::from("."));

    let pi: PreparedItems = read_yaml(&pi_path);
    let en: Entities      = read_yaml(&en_path);
    let ru: Rules         = read_yaml(&ru_path);

    std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| {
        eprintln!("error: cannot create output directory {}: {}", out_dir.display(), e);
        process::exit(1);
    });

    let constraint_map: HashMap<&str, &ConstraintDef> =
        pi.constraints.iter().map(|c| (c.id.as_str(), c)).collect();
    let temporal_map: HashMap<&str, &TemporalDef> =
        pi.temporal.iter().map(|t| (t.id.as_str(), t)).collect();
    let entity_field_map: HashMap<&str, Vec<&FieldDef>> =
        en.entities.iter().map(|e| (e.id.as_str(), e.fields.iter().collect())).collect();

    // Group rules by entity then by trigger.
    let mut by_entity: HashMap<String, Vec<&RuleDef>> = HashMap::new();
    for rule in &ru.rules {
        by_entity.entry(rule.entity.clone()).or_default().push(rule);
    }
    let mut entity_names: Vec<String> = by_entity.keys().cloned().collect();
    entity_names.sort();

    // ---------------------------------------------------------------- //
    // Certo source output
    // ---------------------------------------------------------------- //
    emit_cto_files(&pi, &by_entity, &entity_names, &entity_field_map,
                   &constraint_map, &temporal_map, &out_dir);

    // ---------------------------------------------------------------- //
    // PL/pgSQL output (optional)
    // ---------------------------------------------------------------- //
    if let Some(sql_out) = sql_dir {
        emit_sql_files(&pi, &ru, &by_entity, &entity_names,
                       &constraint_map, &temporal_map, &sql_out);
    }
}

// ================================================================== //
// Certo source emitter
// ================================================================== //

fn emit_cto_files(
    pi:               &PreparedItems,
    by_entity:        &HashMap<String, Vec<&RuleDef>>,
    entity_names:     &[String],
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
    constraint_map:   &HashMap<&str, &ConstraintDef>,
    temporal_map:     &HashMap<&str, &TemporalDef>,
    out_dir:          &Path,
) {
    // constraints.cto
    if !pi.constraints.is_empty() {
        let mut src = String::from("module Constraints\n\n");
        for c in &pi.constraints {
            let cond = emit_certo_condition(&c.condition, constraint_map, temporal_map);
            src.push_str(&format!("constraint {} = {}\n", snake_to_pascal(&c.id), cond));
        }
        write_output(&out_dir.join("constraints.cto"), &src);
    }

    // temporals.cto
    if !pi.temporal.is_empty() {
        let mut src = String::from("module Temporals\n\n");
        for t in &pi.temporal {
            let (method, amount) = temporal_to_duration(t);
            src.push_str(&format!(
                "temporal {} = Duration.{}({})\n",
                snake_to_pascal(&t.id), method, amount,
            ));
        }
        write_output(&out_dir.join("temporals.cto"), &src);
    }

    // per-entity validator files
    let mut cto_count = 0usize;
    for entity_name in entity_names {
        let rules = &by_entity[entity_name];
        let module_name = format!("{}Validators", entity_name);
        let mut src = format!("module {}\n\n", module_name);

        let mut triggers: Vec<String> = Vec::new();
        let mut by_trigger: HashMap<String, Vec<&RuleDef>> = HashMap::new();
        for rule in rules.iter() {
            if !by_trigger.contains_key(&rule.trigger) {
                triggers.push(rule.trigger.clone());
            }
            by_trigger.entry(rule.trigger.clone()).or_default().push(rule);
        }

        for trigger in &triggers {
            let trigger_rules = &by_trigger[trigger];
            let validator_name = format!("{}{}", entity_name, trigger_pascal(trigger));
            let errors_type    = format!("{}Error", entity_name);

            let context_entities = infer_context_entities(
                trigger_rules, entity_name, entity_field_map,
            );

            src.push_str(&format!(
                "validator {} for {} errors {} {{\n",
                validator_name, entity_name, errors_type,
            ));

            if !context_entities.is_empty() {
                src.push_str("    context {\n");
                for ctx_entity in &context_entities {
                    src.push_str(&format!("        {}: {}\n", lowercase_first(ctx_entity), ctx_entity));
                }
                src.push_str("    }\n");
            }

            let rule_ids: std::collections::HashSet<&str> =
                trigger_rules.iter().map(|r| r.id.as_str()).collect();

            for rule in trigger_rules.iter() {
                src.push_str(&format!("    rule {} {{\n", rule.id));
                for dep in &rule.depends_on {
                    if rule_ids.contains(dep.as_str()) {
                        src.push_str(&format!("        after {}\n", dep));
                    }
                }
                if let Some(ov) = &rule.overrides {
                    if rule_ids.contains(ov.as_str()) {
                        src.push_str(&format!("        overrides {}\n", ov));
                    }
                }
                if let Some(pri) = rule.priority {
                    src.push_str(&format!("        priority {}\n", pri));
                }
                let cond = emit_certo_condition(&rule.condition, constraint_map, temporal_map);
                src.push_str(&format!("        require {}\n", cond));
                src.push_str(&format!("        else {}.{}\n", errors_type, rule.error.code));
                src.push_str("    }\n");
            }
            src.push_str("}\n\n");
        }

        let path = out_dir.join(format!("{}-validators.cto", to_kebab(entity_name)));
        write_output(&path, src.trim_end());
        append_newline(&path);
        cto_count += 1;
    }

    eprintln!(
        "generated {} certo file(s) in {}",
        cto_count + if !pi.constraints.is_empty() { 1 } else { 0 }
                  + if !pi.temporal.is_empty()     { 1 } else { 0 },
        out_dir.display()
    );
}

// ================================================================== //
// PL/pgSQL emitter
// ================================================================== //

fn emit_sql_files(
    pi:             &PreparedItems,
    ru:             &Rules,
    by_entity:      &HashMap<String, Vec<&RuleDef>>,
    entity_names:   &[String],
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
    sql_dir:        &Path,
) {
    let validators_dir = sql_dir.join("005_validators");
    std::fs::create_dir_all(&validators_dir).unwrap_or_else(|e| {
        eprintln!("error: cannot create {}: {}", validators_dir.display(), e);
        process::exit(1);
    });

    // 004_rule_types.sql — shared infrastructure
    write_output(&sql_dir.join("004_rule_types.sql"), &build_rule_types());

    // 005_validators/<entity>_<trigger>.sql — one per entity+trigger
    let mut all_groups: Vec<(String, String, Vec<&RuleDef>)> = Vec::new();
    for entity_name in entity_names {
        let rules = &by_entity[entity_name];
        let mut triggers: Vec<String> = Vec::new();
        let mut by_trigger: HashMap<String, Vec<&RuleDef>> = HashMap::new();
        for rule in rules.iter() {
            if !by_trigger.contains_key(&rule.trigger) {
                triggers.push(rule.trigger.clone());
            }
            by_trigger.entry(rule.trigger.clone()).or_default().push(rule);
        }
        for trigger in triggers {
            let group = by_trigger.remove(&trigger).unwrap();
            let filename = format!("{}_{}.sql", to_snake(entity_name), trigger);
            let sql = build_validator_fn(entity_name, &trigger, &group, constraint_map, temporal_map);
            write_output(&validators_dir.join(&filename), &sql);
            all_groups.push((entity_name.clone(), trigger, group));
        }
    }

    // 006_rule_triggers.sql — trigger wiring
    write_output(&sql_dir.join("006_rule_triggers.sql"),
                 &build_rule_triggers(&all_groups));

    // 007_api_helpers.sql — preflight wrappers
    write_output(&sql_dir.join("007_api_helpers.sql"),
                 &build_api_helpers(&all_groups));

    eprintln!("generated {} sql file(s) in {}", all_groups.len() + 3, sql_dir.display());
}

// ------------------------------------------------------------------ //
// 004_rule_types.sql
// ------------------------------------------------------------------ //

fn build_rule_types() -> String {
    let ts = timestamp_comment();
    format!(
r#"-- =============================================================================
-- Rule Validation Infrastructure
-- Generated: {ts}
-- DO NOT EDIT — regenerate from YAML definitions
-- =============================================================================

-- raise_rule_violation(code, message)
-- Raises a structured exception: 'RULE_CODE|Human readable message'
CREATE OR REPLACE FUNCTION raise_rule_violation(
    p_code    TEXT,
    p_message TEXT
) RETURNS VOID AS $$
BEGIN
    RAISE EXCEPTION USING
        ERRCODE = 'P0001',
        MESSAGE = p_code || '|' || p_message,
        HINT    = p_message;
END;
$$ LANGUAGE plpgsql;

-- current_rule_context()
-- Returns JSONB context set by the application via SET LOCAL.
--   SET LOCAL rule.user_id   = '<uuid>';
--   SET LOCAL rule.user_role = 'ADMIN';
CREATE OR REPLACE FUNCTION current_rule_context()
RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object(
        'user_id',   current_setting('rule.user_id',   TRUE),
        'user_role', current_setting('rule.user_role', TRUE)
    );
END;
$$ LANGUAGE plpgsql STABLE;

-- record_age_days(ts) → NUMERIC
-- How many days ago a timestamp occurred.
CREATE OR REPLACE FUNCTION record_age_days(ts TIMESTAMPTZ)
RETURNS NUMERIC AS $$
BEGIN
    RETURN EXTRACT(EPOCH FROM (NOW() - ts)) / 86400.0;
END;
$$ LANGUAGE plpgsql STABLE;
"#
    )
}

// ------------------------------------------------------------------ //
// 005_validators/<entity>_<trigger>.sql
// ------------------------------------------------------------------ //

fn build_validator_fn(
    entity:         &str,
    trigger:        &str,
    rules:          &[&RuleDef],
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    let ts        = timestamp_comment();
    let func_name = format!("validate_{}_{}", to_plural_snake(entity), trigger);
    let ordered   = topo_sort_rules(rules);

    // Sets of rule IDs used for dependency tracking.
    let depended_on: std::collections::HashSet<&str> = ordered.iter()
        .flat_map(|r| r.depends_on.iter().map(|d| d.as_str()))
        .collect();
    // Map from overridden rule ID → overriding rule ID.
    let override_map: HashMap<&str, &str> = ordered.iter()
        .filter_map(|r| r.overrides.as_deref().map(|ov| (ov, r.id.as_str())))
        .collect();
    // Override rules (the ones doing the overriding) — evaluate condition, set flag, don't raise.
    let is_override_rule: std::collections::HashSet<&str> = ordered.iter()
        .filter(|r| r.overrides.is_some())
        .map(|r| r.id.as_str())
        .collect();

    let mut sb = String::new();

    // Header
    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- Validator: {entity}.{trigger}\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n"
    ));

    // Rule index comment
    sb.push_str(&format!("-- Rules enforced ({}):\n", ordered.len()));
    for r in &ordered {
        sb.push_str(&format!("--   {}: {}\n", r.id, r.description));
    }
    sb.push('\n');

    // Function signature
    sb.push_str(&format!(
        "CREATE OR REPLACE FUNCTION {}(\n    p_record  JSONB,\n    p_context JSONB\n) RETURNS VOID AS $$\nDECLARE\n",
        func_name
    ));

    // Variable declarations
    sb.push_str("    v_user_role TEXT;\n");
    for id in &depended_on {
        sb.push_str(&format!("    v_{}_passed BOOLEAN := FALSE;\n", to_var(id)));
    }
    for r in ordered.iter().filter(|r| r.overrides.is_some()) {
        sb.push_str(&format!("    v_{}_active BOOLEAN := FALSE;\n", to_var(&r.id)));
    }
    sb.push_str("BEGIN\n");
    sb.push_str("    v_user_role := p_context->>'user_role';\n\n");

    // Emit override rules first (they set flags, never raise)
    for rule in ordered.iter().filter(|r| is_override_rule.contains(r.id.as_str())) {
        let ov_target = rule.overrides.as_deref().unwrap_or("");
        sb.push_str(&format!(
            "    -- {} (override — suspends '{}' when true)\n",
            rule.description, ov_target
        ));
        let cond = emit_sql_condition(&rule.condition, constraint_map, temporal_map);
        sb.push_str(&format!("    IF {} THEN\n", cond));
        sb.push_str(&format!("        v_{}_active := TRUE;\n    END IF;\n\n", to_var(&rule.id)));
    }

    // Emit regular rules in topological order (skip override rules — already emitted above)
    for rule in ordered.iter().filter(|r| !is_override_rule.contains(r.id.as_str())) {
        let is_depended = depended_on.contains(rule.id.as_str());
        let is_overridden = override_map.contains_key(rule.id.as_str());

        sb.push_str(&format!("    -- {}\n", rule.description));

        // depends_on guard
        let dep_vars: Vec<String> = rule.depends_on.iter()
            .filter(|d| ordered.iter().any(|r| &r.id == *d))
            .map(|d| format!("v_{}_passed", to_var(d)))
            .collect();
        let depth = if dep_vars.is_empty() { 0usize } else { 1 };
        if !dep_vars.is_empty() {
            sb.push_str(&format!("    IF {} THEN\n", dep_vars.join(" AND ")));
        }

        let ind = "    ".repeat(depth + 1);

        // overrides guard
        if is_overridden {
            let overriding = override_map[rule.id.as_str()];
            sb.push_str(&format!("{}IF NOT v_{}_active THEN\n", ind, to_var(overriding)));
        }
        let inner_ind = if is_overridden {
            format!("{}    ", ind)
        } else {
            ind.clone()
        };

        let cond = emit_sql_condition(&rule.condition, constraint_map, temporal_map);
        sb.push_str(&format!("{}IF NOT ({}) THEN\n", inner_ind, cond));
        sb.push_str(&format!(
            "{}    PERFORM raise_rule_violation('{}', '{}');\n",
            inner_ind,
            rule.error.code,
            rule.error.message.replace('\'', "''"),
        ));
        sb.push_str(&format!("{}END IF;\n", inner_ind));

        if is_overridden {
            sb.push_str(&format!("{}END IF;\n", ind));
        }

        // Record pass flag after the check (if depended on by others)
        if is_depended {
            sb.push_str(&format!(
                "{}v_{}_passed := {};\n",
                "    ".repeat(depth + 1),
                to_var(&rule.id),
                cond_for_pass(&rule.condition, constraint_map, temporal_map),
            ));
        }

        if !dep_vars.is_empty() {
            sb.push_str("    END IF;\n");
        }
        sb.push('\n');
    }

    sb.push_str("END;\n$$ LANGUAGE plpgsql;\n\n");

    // Convenience single-arg wrapper
    sb.push_str(&format!(
        "-- Convenience wrapper reads context from session settings.\n\
         CREATE OR REPLACE FUNCTION {}(p_record JSONB)\n\
         RETURNS VOID AS $$\n\
         BEGIN\n\
             PERFORM {}(p_record, current_rule_context());\n\
         END;\n\
         $$ LANGUAGE plpgsql;\n",
        func_name, func_name
    ));

    sb
}

// ------------------------------------------------------------------ //
// 006_rule_triggers.sql
// ------------------------------------------------------------------ //

fn build_rule_triggers(groups: &[(String, String, Vec<&RuleDef>)]) -> String {
    let ts = timestamp_comment();
    let mut sb = String::new();

    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- Rule Triggers\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n"
    ));

    // One trigger function per entity.
    let mut by_entity: HashMap<&str, Vec<&str>> = HashMap::new();
    for (entity, trigger, _) in groups {
        by_entity.entry(entity.as_str()).or_default().push(trigger.as_str());
    }

    let mut entity_names: Vec<&str> = by_entity.keys().copied().collect();
    entity_names.sort();

    for entity in entity_names {
        let triggers = &by_entity[entity];
        let table_name  = to_plural_snake(entity);
        let trg_fn_name = format!("trg_validate_{}", table_name);
        let func_name_base = format!("validate_{}", table_name);

        sb.push_str(&format!("-- {entity}\n"));
        sb.push_str(&format!(
            "CREATE OR REPLACE FUNCTION {trg_fn_name}()\n\
             RETURNS TRIGGER AS $$\n\
             DECLARE\n\
                 v_record  JSONB;\n\
                 v_context JSONB;\n\
             BEGIN\n\
                 v_record  := row_to_json(NEW)::JSONB;\n\
                 v_context := current_rule_context();\n"
        ));

        // INSERT triggers
        let has_create = triggers.iter().any(|t| *t == "create");
        if has_create {
            sb.push_str(&format!(
                "    IF TG_OP = 'INSERT' THEN\n        PERFORM {func_name_base}_create(v_record, v_context);\n"
            ));
        } else {
            sb.push_str("    IF TG_OP = 'INSERT' THEN\n        NULL;\n");
        }

        // Named action triggers (e.g. submit, void, apply_discount) are not
        // wired here — they are called explicitly via preflight_* API helpers
        // from the application before committing DML. The DB trigger covers
        // only the generic create/update cases as a safety net.
        let has_plain_update = triggers.iter().any(|t| *t == "update");

        if has_plain_update {
            sb.push_str("    ELSIF TG_OP = 'UPDATE' THEN\n");
            sb.push_str(&format!(
                "        PERFORM {func_name_base}_update(v_record, v_context);\n"
            ));
        } else {
            sb.push_str("    ELSIF TG_OP = 'UPDATE' THEN\n        NULL; -- named-action validators called via preflight_* API helpers\n");
        }

        sb.push_str(
            "    END IF;\n    RETURN NEW;\nEND;\n$$ LANGUAGE plpgsql;\n\n"
        );

        sb.push_str(&format!(
            "DROP TRIGGER IF EXISTS {trg_fn_name} ON {table_name};\n\
             CREATE TRIGGER {trg_fn_name}\n\
                 BEFORE INSERT OR UPDATE ON {table_name}\n\
                 FOR EACH ROW\n\
                 EXECUTE FUNCTION {trg_fn_name}();\n\n\
             -------------------------------------------------------------------------------\n\n"
        ));
    }

    sb
}

// ------------------------------------------------------------------ //
// 007_api_helpers.sql
// ------------------------------------------------------------------ //

fn build_api_helpers(groups: &[(String, String, Vec<&RuleDef>)]) -> String {
    let ts = timestamp_comment();
    let mut sb = String::new();

    sb.push_str(&format!(
        "-- =============================================================================\n\
         -- API Helper Functions\n\
         -- Generated: {ts}\n\
         -- DO NOT EDIT — regenerate from YAML definitions\n\
         -- =============================================================================\n\n\
         -- Pre-flight check wrappers — return JSONB instead of raising.\n\
         -- Use from REST handlers before committing DML.\n\n"
    ));

    // Generic validate_safe wrapper
    sb.push_str(
        "CREATE OR REPLACE FUNCTION validate_safe(\n\
             p_func    TEXT,\n\
             p_record  JSONB,\n\
             p_context JSONB\n\
         ) RETURNS JSONB AS $$\n\
         DECLARE\n\
             v_parts TEXT[];\n\
         BEGIN\n\
             EXECUTE format('SELECT %I($1, $2)', p_func)\n\
                 USING p_record, p_context;\n\
             RETURN jsonb_build_object('valid', TRUE, 'violations', '[]'::JSONB);\n\
         EXCEPTION WHEN OTHERS THEN\n\
             v_parts := string_to_array(SQLERRM, '|');\n\
             RETURN jsonb_build_object(\n\
                 'valid', FALSE,\n\
                 'violations', jsonb_build_array(jsonb_build_object(\n\
                     'code',    v_parts[1],\n\
                     'message', COALESCE(v_parts[2], SQLERRM)\n\
                 ))\n\
             );\n\
         END;\n\
         $$ LANGUAGE plpgsql;\n\n"
    );

    for (entity, trigger, _) in groups {
        let table   = to_plural_snake(entity);
        let fn_name = format!("validate_{}_{}", table, trigger);
        let pf_name = format!("preflight_{}_{}", table, trigger);

        sb.push_str(&format!(
            "-- Pre-flight: {entity}.{trigger}\n\
             CREATE OR REPLACE FUNCTION {pf_name}(\n\
                 p_record  JSONB,\n\
                 p_context JSONB DEFAULT current_rule_context()\n\
             ) RETURNS JSONB AS $$\n\
             BEGIN\n\
                 RETURN validate_safe('{fn_name}', p_record, p_context);\n\
             END;\n\
             $$ LANGUAGE plpgsql;\n\n"
        ));
    }

    sb
}

// ================================================================== //
// Condition → Certo expression
// ================================================================== //

fn emit_certo_condition(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    match node {
        ConditionNode::ConstraintRef { constraint } => snake_to_pascal(constraint),

        ConditionNode::TemporalRef { temporal, field, operator } => {
            let tname = snake_to_pascal(temporal);
            match operator.as_str() {
                "age_lt"  => format!("{}.age < {}", field, tname),
                "age_gt"  => format!("{}.age > {}", field, tname),
                "age_lte" => format!("{}.age <= {}", field, tname),
                "age_gte" => format!("{}.age >= {}", field, tname),
                _         => format!("{}.age < {}", field, tname),
            }
        }

        ConditionNode::All { all } => {
            let parts: Vec<String> = all.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" and ")
        }

        ConditionNode::Any { any } => {
            let parts: Vec<String> = any.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" or ")
        }

        ConditionNode::None { none } => {
            let parts: Vec<String> = none.iter()
                .map(|c| emit_certo_condition(c, constraint_map, temporal_map))
                .collect();
            let inner = if parts.len() == 1 { parts.into_iter().next().unwrap() }
                        else { parts.iter().map(|p| certo_paren(p)).collect::<Vec<_>>().join(" or ") };
            format!("not ({})", inner)
        }

        ConditionNode::Leaf { field, operator, value, .. } => {
            match operator.as_str() {
                "eq"          => format!("{} == {}", field, certo_val(value)),
                "not_eq"      => format!("{} != {}", field, certo_val(value)),
                "gt"          => format!("{} > {}", field, certo_val(value)),
                "gte"         => format!("{} >= {}", field, certo_val(value)),
                "lt"          => format!("{} < {}", field, certo_val(value)),
                "lte"         => format!("{} <= {}", field, certo_val(value)),
                "in"          => format!("{} in {}", field, certo_val(value)),
                "not_in"      => format!("{} not in {}", field, certo_val(value)),
                "exists"      => format!("{}.isSome()", field),
                "not_exists"  => format!("{}.isNone()", field),
                "matches"     => format!("Text.matches({}, {})", field, certo_val(value)),
                "not_matches" => format!("not Text.matches({}, {})", field, certo_val(value)),
                "contains"    => format!("Text.contains({}, {})", field, certo_val(value)),
                "starts_with" => format!("Text.startsWith({}, {})", field, certo_val(value)),
                other         => format!("{} {} {}", field, other, certo_val(value)),
            }
        }
    }
}

// ================================================================== //
// Condition → SQL expression
// ================================================================== //

fn emit_sql_condition(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    match node {
        ConditionNode::ConstraintRef { constraint } => {
            // Inline the constraint condition.
            if let Some(c) = constraint_map.get(constraint.as_str()) {
                emit_sql_condition(&c.condition, constraint_map, temporal_map)
            } else {
                format!("/* unknown constraint: {} */ TRUE", constraint)
            }
        }

        ConditionNode::TemporalRef { temporal, field, operator } => {
            let days = if let Some(t) = temporal_map.get(temporal.as_str()) {
                let (_, amount) = temporal_to_duration(t);
                amount
            } else {
                30
            };
            // JSONB ->> returns text; cast to TIMESTAMPTZ for record_age_days().
            let col = format!("{}::TIMESTAMPTZ", field_to_sql_col(field));
            match operator.as_str() {
                "age_lt"  => format!("record_age_days({}) < {}", col, days),
                "age_gt"  => format!("record_age_days({}) > {}", col, days),
                "age_lte" => format!("record_age_days({}) <= {}", col, days),
                "age_gte" => format!("record_age_days({}) >= {}", col, days),
                _         => format!("record_age_days({}) < {}", col, days),
            }
        }

        ConditionNode::All { all } => {
            let parts: Vec<String> = all.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\nAND ")
        }

        ConditionNode::Any { any } => {
            let parts: Vec<String> = any.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            if parts.len() == 1 { return parts.into_iter().next().unwrap(); }
            parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\n    OR ")
        }

        ConditionNode::None { none } => {
            let parts: Vec<String> = none.iter()
                .map(|c| emit_sql_condition(c, constraint_map, temporal_map))
                .collect();
            let inner = if parts.len() == 1 { parts.into_iter().next().unwrap() }
                        else { parts.iter().map(|p| sql_paren(p)).collect::<Vec<_>>().join("\n    OR ") };
            format!("NOT ({})", inner)
        }

        ConditionNode::Leaf { field, operator, value, .. } => {
            let col = field_to_sql_col(field);
            let typed_col = sql_cast_col(&col, value);
            match operator.as_str() {
                "eq"          => format!("{} = {}", typed_col, sql_val(value)),
                "not_eq"      => format!("{} != {}", typed_col, sql_val(value)),
                "gt"          => format!("{} > {}", typed_col, sql_val(value)),
                "gte"         => format!("{} >= {}", typed_col, sql_val(value)),
                "lt"          => format!("{} < {}", typed_col, sql_val(value)),
                "lte"         => format!("{} <= {}", typed_col, sql_val(value)),
                "in"          => format!("{} IN ({})", col, sql_list_val(value)),
                "not_in"      => format!("{} NOT IN ({})", col, sql_list_val(value)),
                "exists"      => format!("{} IS NOT NULL", col),
                "not_exists"  => format!("{} IS NULL", col),
                "matches"     => format!("{} ~ {}", col, sql_val(value)),
                "not_matches" => format!("{} !~ {}", col, sql_val(value)),
                "contains"    => format!("{} ILIKE '%%' || {} || '%%'", col, sql_val(value)),
                "starts_with" => format!("{} ILIKE {} || '%%'", col, sql_val(value)),
                other         => format!("{} {} {}", typed_col, other.to_uppercase(), sql_val(value)),
            }
        }
    }
}

/// Used to record the pass flag — re-emits the raw condition so we can assign `flag := <cond>`.
/// For complex conditions we wrap in a subexpression check.
fn cond_for_pass(
    node:           &ConditionNode,
    constraint_map: &HashMap<&str, &ConstraintDef>,
    temporal_map:   &HashMap<&str, &TemporalDef>,
) -> String {
    let cond = emit_sql_condition(node, constraint_map, temporal_map);
    // Already a boolean expression — assign directly.
    format!("({})", cond)
}

// ================================================================== //
// Field path → SQL column expression
// ================================================================== //

/// Returns a bare JSONB ->> accessor with no type cast.
/// The caller applies a cast based on the comparison value type.
///
/// `order.total` → `(p_record->>'total')`
/// `user.role`   → `(p_context->>'user_role')`
/// `status`      → `(p_record->>'status')`
fn field_to_sql_col(field: &str) -> String {
    let ctx_roots = ["user", "operator", "actor"];
    if let Some(dot) = field.find('.') {
        let root = &field[..dot];
        let rest = field[dot + 1..].replace('.', "_");
        if ctx_roots.contains(&root) {
            return format!("(p_context->>'{}_{}')", root, rest);
        }
        let col = camel_to_snake(&rest);
        return format!("(p_record->>'{}')", col);
    }
    let col = camel_to_snake(field);
    format!("(p_record->>'{}')", col)
}

/// Wraps a bare JSONB accessor with the appropriate Postgres cast
/// based on what we're comparing it to.
fn sql_cast_col(col: &str, value: &Option<serde_yaml::Value>) -> String {
    match value {
        Some(serde_yaml::Value::Number(_)) => format!("{}::NUMERIC", col),
        Some(serde_yaml::Value::Bool(_))   => format!("{}::BOOLEAN", col),
        _                                   => col.to_string(),
    }
}

// ================================================================== //
// Value formatting
// ================================================================== //

fn certo_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        None => "null".into(),
        Some(serde_yaml::Value::Bool(b))   => b.to_string(),
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Null)      => "null".into(),
        Some(serde_yaml::Value::String(s)) => {
            if s.chars().all(|c| c.is_uppercase() || c == '_')
                || s.contains('.')
                || s.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
            {
                s.clone()
            } else {
                format!("\"{}\"", s)
            }
        }
        Some(other) => format!("{:?}", other),
    }
}

fn sql_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        None => "NULL".into(),
        Some(serde_yaml::Value::Bool(b))   => if *b { "TRUE".into() } else { "FALSE".into() },
        Some(serde_yaml::Value::Number(n)) => n.to_string(),
        Some(serde_yaml::Value::Null)      => "NULL".into(),
        Some(serde_yaml::Value::String(s)) => {
            // Lowercase identifier or dot-path = field reference, not a string literal.
            let is_field_ref = s.chars().next().map(|c| c.is_lowercase()).unwrap_or(false)
                && s.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_');
            if is_field_ref {
                // Numeric field references need a cast so comparisons work correctly.
                format!("{}::NUMERIC", field_to_sql_col(s))
            } else {
                format!("'{}'", s.replace('\'', "''"))
            }
        }
        Some(other) => format!("'{:?}'", other),
    }
}

fn sql_list_val(v: &Option<serde_yaml::Value>) -> String {
    match v {
        Some(serde_yaml::Value::Sequence(items)) => {
            items.iter()
                .map(|item| {
                    let wrapped = Some(item.clone());
                    sql_val(&wrapped)
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
        other => sql_val(other),
    }
}

fn certo_paren(s: &str) -> String {
    if s.contains(" and ") || s.contains(" or ") { format!("({})", s) } else { s.to_string() }
}

fn sql_paren(s: &str) -> String {
    if s.contains(" AND ") || s.contains(" OR ") || s.contains('\n') {
        format!("({})", s)
    } else {
        s.to_string()
    }
}

// ================================================================== //
// Topological sort (respects depends_on within the group)
// ================================================================== //

fn topo_sort_rules<'a>(rules: &[&'a RuleDef]) -> Vec<&'a RuleDef> {
    let idx: HashMap<&str, usize> = rules.iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let mut visited = vec![false; rules.len()];
    let mut order: Vec<usize> = Vec::with_capacity(rules.len());

    fn visit(i: usize, rules: &[&RuleDef], idx: &HashMap<&str, usize>,
             visited: &mut Vec<bool>, order: &mut Vec<usize>) {
        if visited[i] { return; }
        visited[i] = true;
        for dep in &rules[i].depends_on {
            if let Some(&j) = idx.get(dep.as_str()) { visit(j, rules, idx, visited, order); }
        }
        // Also visit overriding rules before the rule they override.
        for (k, r) in rules.iter().enumerate() {
            if r.overrides.as_deref() == Some(rules[i].id.as_str()) {
                visit(k, rules, idx, visited, order);
            }
        }
        order.push(i);
    }

    for i in 0..rules.len() { visit(i, rules, &idx, &mut visited, &mut order); }
    order.iter().map(|&i| rules[i]).collect()
}

// ================================================================== //
// String helpers
// ================================================================== //

fn read_yaml<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {}", path.display(), e);
        process::exit(1);
    });
    serde_yaml::from_str(&src).unwrap_or_else(|e| {
        eprintln!("error: cannot parse {}: {}", path.display(), e);
        process::exit(1);
    })
}

fn write_output(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap_or_else(|e| {
        eprintln!("error: cannot write {}: {}", path.display(), e);
        process::exit(1);
    });
    eprintln!("wrote {}", path.display());
}

fn append_newline(path: &Path) {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path)
        .and_then(|mut f| f.write_all(b"\n")).ok();
}

fn next_path(args: &[String], i: usize, flag: &str) -> PathBuf {
    PathBuf::from(args.get(i).unwrap_or_else(|| die(&format!("{} requires a path", flag), 2)))
}

fn die(msg: &str, code: i32) -> ! {
    eprintln!("error: {}", msg);
    process::exit(code);
}

fn timestamp_comment() -> String {
    // A fixed-format UTC-like timestamp without pulling in chrono.
    // In practice the Rust std doesn't expose UTC time nicely, so we emit a placeholder.
    "see git history for generation date".to_string()
}

/// snake_case → PascalCase
fn snake_to_pascal(s: &str) -> String {
    s.split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }).collect()
}

/// PascalCase/camelCase → kebab-case
fn to_kebab(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 { out.push('-'); }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// PascalCase/camelCase → snake_case
fn to_snake(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 { out.push('_'); }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// camelCase field name → snake_case column name
fn camel_to_snake(s: &str) -> String {
    to_snake(s)
}

/// PascalCase entity name → plural snake_case table name (naïve: append 's' unless already ends in 's')
fn to_plural_snake(entity: &str) -> String {
    let snake = to_snake(entity);
    if snake.ends_with('s') { snake } else { format!("{}s", snake) }
}

/// trigger name → PascalCase suffix for validator name
fn trigger_pascal(t: &str) -> String {
    t.split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }).collect()
}

/// Rule id → SQL variable name (replace - with _)
fn to_var(id: &str) -> String {
    id.replace('-', "_")
}

fn lowercase_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|ch| ch.to_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

fn capitalize_first(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
}

fn temporal_to_duration(t: &TemporalDef) -> (&'static str, u64) {
    match t.unit.as_str() {
        "minutes" => ("minutes", t.duration),
        "hours"   => ("hours",   t.duration),
        "days"    => ("days",    t.duration),
        "weeks"   => ("days",    t.duration * 7),
        "months"  => ("days",    t.duration * 30),
        "years"   => ("days",    t.duration * 365),
        _         => ("days",    t.duration),
    }
}

// ================================================================== //
// Context inference
// ================================================================== //

fn infer_context_entities<'a>(
    rules:            &[&RuleDef],
    primary_entity:   &str,
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
) -> Vec<String> {
    let primary_lower = lowercase_first(primary_entity);
    let mut seen: Vec<String> = Vec::new();
    let mut seen_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rule in rules {
        collect_field_roots(&rule.condition, &primary_lower, entity_field_map, &mut seen, &mut seen_set);
    }
    seen
}

fn collect_field_roots(
    node:             &ConditionNode,
    primary_var:      &str,
    entity_field_map: &HashMap<&str, Vec<&FieldDef>>,
    seen:             &mut Vec<String>,
    seen_set:         &mut std::collections::HashSet<String>,
) {
    match node {
        ConditionNode::Leaf { field, .. } => {
            if let Some(root) = field.split('.').next() {
                if root != primary_var && !seen_set.contains(root) {
                    let entity_name = capitalize_first(root);
                    if entity_field_map.contains_key(entity_name.as_str()) {
                        seen_set.insert(root.to_string());
                        seen.push(entity_name);
                    }
                }
            }
        }
        ConditionNode::All  { all  } => all .iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        ConditionNode::Any  { any  } => any .iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        ConditionNode::None { none } => none.iter().for_each(|c| collect_field_roots(c, primary_var, entity_field_map, seen, seen_set)),
        _ => {}
    }
}
