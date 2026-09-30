//! Adopt an existing database: turn its introspected schema into `schema.sdl`
//! plus a baseline migration recorded as already applied.
//!
//! Pipeline: introspect -> translate live defaults/CHECKs (`pgexpr`) ->
//! sanitize (drop what SDL cannot express, and everything that depends on it)
//! -> print SDL -> recompile and require the exact same IR.
//!
//! The baseline migration holds the real `CREATE` statements, so an empty
//! database can be built from the history; on the adopted database it is only
//! *recorded* as applied, never run. Nothing that could not be adopted is
//! guessed: each omission is listed so you can decide what to do about it.

use crate::drift::{self, DriftItem};
use crate::error::{io, RunnerError};
use crate::exec::Executor;
use crate::introspect::LiveSchema;
use crate::migration::{create, list};
use crate::pgexpr::{translate_check_for, translate_default_for};
use certo_sql::Dialect;
use crate::project::Project;
use certo_diagnostics::render_all;
use certo_sdl::{compile, parse, to_sdl, Builtin, ExprIR, SchemaIR, TypeIR};
use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Debug, Clone, Default)]
pub struct AdoptOptions {
    /// Report what would be adopted; write nothing, record nothing.
    pub dry_run: bool,
    /// Overwrite a `schema.sdl` that already has declarations.
    pub force: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Counts {
    pub tables: usize,
    pub columns: usize,
    pub enums: usize,
    pub types: usize,
    pub sequences: usize,
    pub indexes: usize,
    pub constraints: usize,
}

#[derive(Debug, Clone)]
pub struct AdoptReport {
    pub dry_run: bool,
    pub schema_sdl: String,
    pub adopted: Counts,
    /// Everything in the database that is not in `schema_sdl`, and why.
    pub omissions: Vec<String>,
    /// Label of the baseline migration (`None` for a dry run).
    pub migration: Option<String>,
    /// Differences that remain between the database and the adopted schema
    /// (for example a default SDL cannot express). Empty when it matches.
    pub known_drift: Vec<DriftItem>,
}

/// The result of `prepare`: the schema and what was left out.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub ir: SchemaIR,
    pub sdl: String,
    pub omissions: Vec<String>,
    pub counts: Counts,
}

fn valid_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(f) if f.is_ascii_alphabetic()) && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Does this name work as an SDL table/enum/type name?
fn usable_name(s: &str) -> bool { valid_ident(s) && Builtin::from_name(s).is_none() }

fn columns_in(e: &ExprIR, out: &mut Vec<String>) {
    match e {
        ExprIR::Column { name } => out.push(name.clone()),
        ExprIR::Call { args, .. } => args.iter().for_each(|a| columns_in(a, out)),
        ExprIR::Binary { lhs, rhs, .. } => {
            columns_in(lhs, out);
            columns_in(rhs, out);
        }
        ExprIR::Not { expr } | ExprIR::IsNull { expr, .. } => columns_in(expr, out),
        ExprIR::In { expr, list, .. } => {
            columns_in(expr, out);
            list.iter().for_each(|e| columns_in(e, out));
        }
        ExprIR::Number { .. }
        | ExprIR::Decimal { .. }
        | ExprIR::String { .. }
        | ExprIR::Bool { .. }
        | ExprIR::EnumVariant { .. }
        | ExprIR::NextVal { .. }
        | ExprIR::Raw { .. } => {}
    }
}

/// Remove what SDL cannot express, repeating until nothing more depends on a
/// removed object. Every removal is recorded in `om`.
fn sanitize(ir: &mut SchemaIR, om: &mut Vec<String>) {
    // names first
    ir.enums.retain(|e| {
        let ok = usable_name(&e.name) && !e.variants.is_empty() && e.variants.iter().all(|v| valid_ident(v));
        if !ok { om.push(format!("enum {} not adopted: its name or a value is not a valid SDL identifier (or it has no values)", e.name)); }
        ok
    });
    ir.types.retain(|t| {
        let ok = usable_name(&t.name);
        if !ok { om.push(format!("type {} not adopted: not a valid SDL name", t.name)); }
        ok
    });
    ir.tables.retain(|t| {
        let ok = usable_name(&t.name);
        if !ok { om.push(format!("table {} not adopted: not a valid SDL name", t.name)); }
        ok
    });

    loop {
        let mut changed = false;
        let enums: HashSet<String> = ir.enums.iter().map(|e| e.name.clone()).collect();
        let types: HashSet<String> = ir.types.iter().map(|t| t.name.clone()).collect();
        let exists = |t: &TypeIR| match t {
            TypeIR::Builtin(_) => true,
            TypeIR::Enum(n) => enums.contains(n),
            TypeIR::Composite(n) => types.contains(n),
        };

        // composite types: fields, then emptiness
        for t in &mut ir.types {
            let before = t.fields.len();
            let name = t.name.clone();
            t.fields.retain(|f| {
                let ok = valid_ident(&f.name) && exists(&f.ty);
                if !ok { om.push(format!("type {name}: field {} not adopted (invalid name or unavailable type)", f.name)); }
                ok
            });
            changed |= t.fields.len() != before;
        }
        let before = ir.types.len();
        ir.types.retain(|t| {
            if t.fields.is_empty() { om.push(format!("type {} not adopted: it has no fields SDL can express", t.name)); }
            !t.fields.is_empty()
        });
        changed |= ir.types.len() != before;

        // columns
        for t in &mut ir.tables {
            let before = t.columns.len();
            let tname = t.name.clone();
            t.columns.retain(|c| {
                let ok = valid_ident(&c.name) && exists(&c.ty);
                if !ok { om.push(format!("column {tname}.{} not adopted (invalid name or unavailable type)", c.name)); }
                ok
            });
            changed |= t.columns.len() != before;
        }

        // foreign keys must satisfy the same rules the SDL compiler enforces
        type ColInfo = (TypeIR, bool, bool, bool); // type, unique, primary key, nullable
        let info: HashMap<String, HashMap<String, ColInfo>> = ir
            .tables
            .iter()
            .map(|t| {
                (t.name.clone(), t.columns.iter().map(|c| (c.name.clone(), (c.ty.clone(), c.unique, c.primary_key, c.nullable))).collect())
            })
            .collect();
        let pk_count: HashMap<String, usize> =
            ir.tables.iter().map(|t| (t.name.clone(), t.columns.iter().filter(|c| c.primary_key).count())).collect();
        for t in &mut ir.tables {
            for c in &mut t.columns {
                let Some(fk) = c.references.clone() else { continue };
                let why = match info.get(&fk.table).and_then(|cols| cols.get(&fk.column)) {
                    None => Some("its target table or column is not adopted".to_string()),
                    Some((ty, unique, pk, _)) => {
                        if !(*unique || (*pk && pk_count[&fk.table] == 1)) {
                            Some(format!("{}.{} is not unique or a sole primary key", fk.table, fk.column))
                        } else if *ty != c.ty {
                            Some("column types differ".to_string())
                        } else if !c.nullable
                            && (fk.on_delete == certo_sdl::ReferentialAction::SetNull || fk.on_update == certo_sdl::ReferentialAction::SetNull)
                        {
                            Some("SET NULL on a NOT NULL column".to_string())
                        } else {
                            None
                        }
                    }
                };
                if let Some(why) = why {
                    om.push(format!("foreign key {}.{} -> {}.{} not adopted: {why}", t.name, c.name, fk.table, fk.column));
                    c.references = None;
                    changed = true;
                }
            }
        }

        // indexes and CHECKs must only use columns that survived
        for t in &mut ir.tables {
            let cols: HashSet<String> = t.columns.iter().map(|c| c.name.clone()).collect();
            let tname = t.name.clone();
            let n = t.indexes.len() + t.constraints.len();
            t.indexes.retain(|i| {
                let ok = valid_ident(&i.name) && !i.columns.is_empty() && i.columns.iter().all(|c| cols.contains(c));
                if !ok { om.push(format!("index {} on {tname} not adopted (invalid name or uses a column that was not adopted)", i.name)); }
                ok
            });
            t.constraints.retain(|k| {
                let mut used = Vec::new();
                columns_in(&k.expr, &mut used);
                let ok = valid_ident(&k.name) && used.iter().all(|c| cols.contains(c));
                if !ok { om.push(format!("constraint {} on {tname} not adopted (invalid name or uses a column that was not adopted)", k.name)); }
                ok
            });
            changed |= t.indexes.len() + t.constraints.len() != n;
        }

        if !changed { break; }
    }

    // SDL constraint names are unique across the schema (PostgreSQL only needs them per table)
    let mut seen: HashSet<String> = HashSet::new();
    for t in &mut ir.tables {
        let tname = t.name.clone();
        t.constraints.retain(|k| {
            let fresh = seen.insert(k.name.clone());
            if !fresh { om.push(format!("constraint {} on {tname} not adopted: the name is already used by another table's constraint", k.name)); }
            fresh
        });
    }
}

/// Live schema -> adoptable SDL, listing everything that had to be left out.
pub fn prepare(live: LiveSchema) -> Result<Prepared, RunnerError> {
    prepare_for(Dialect::Postgres, live)
}

/// `prepare` for a live database of `dialect`.
pub fn prepare_for(dialect: Dialect, live: LiveSchema) -> Result<Prepared, RunnerError> {
    let LiveSchema { mut ir, notes } = live;
    let mut om = notes;
    let enums = ir.enums.clone();

    // Sequences first: SDL keeps tables, enums, types and sequences in one
    // namespace, and a default may only call a sequence that survives.
    let taken: HashSet<String> = ir
        .tables
        .iter()
        .map(|t| t.name.clone())
        .chain(ir.enums.iter().map(|e| e.name.clone()))
        .chain(ir.types.iter().map(|t| t.name.clone()))
        .collect();
    ir.sequences.retain(|s| {
        let ok = usable_name(&s.name) && !taken.contains(&s.name);
        if !ok {
            om.push(format!("sequence {} not adopted: its name is not a valid SDL identifier or is already used by another object", s.name));
        }
        ok
    });
    let sequences = ir.sequences.clone();

    // 1. live SQL text -> SDL expressions
    for t in &mut ir.tables {
        let snapshot = t.columns.clone();
        for c in &mut t.columns {
            if let Some(ExprIR::Raw { sql }) = c.default.clone() {
                match translate_default_for(dialect, &sql, &c.ty, &enums, &sequences) {
                    Ok(e) => c.default = Some(e),
                    Err(why) => {
                        c.default = None;
                        om.push(format!("default of {}.{} not adopted (`{sql}`): {why}", t.name, c.name));
                    }
                }
            }
        }
        let tname = t.name.clone();
        t.constraints = std::mem::take(&mut t.constraints)
            .into_iter()
            .filter_map(|mut k| {
                let ExprIR::Raw { sql } = &k.expr else { return Some(k) };
                match translate_check_for(dialect, sql, &snapshot, &enums) {
                    Ok(e) => {
                        k.expr = e;
                        Some(k)
                    }
                    Err(why) => {
                        om.push(format!("constraint {} on {tname} not adopted (`{sql}`): {why}", k.name));
                        None
                    }
                }
            })
            .collect();
    }

    // 2. canonical order (the database sorts by its own collation; SDL by name)
    ir.tables.sort_by(|a, b| a.name.cmp(&b.name));
    ir.enums.sort_by(|a, b| a.name.cmp(&b.name));
    ir.types.sort_by(|a, b| a.name.cmp(&b.name));
    ir.sequences.sort_by(|a, b| a.name.cmp(&b.name));
    for t in &mut ir.tables {
        t.indexes.sort_by(|a, b| a.name.cmp(&b.name));
        t.constraints.sort_by(|a, b| a.name.cmp(&b.name));
    }

    // 3. drop what SDL cannot express
    sanitize(&mut ir, &mut om);

    // 4. print, recompile, and require the same IR
    let sdl = to_sdl(&ir).map_err(|e| RunnerError::Project(format!("internal error: cannot print the adopted schema: {e}")))?;
    let (compiled, diags) = compile(&sdl);
    let Some(compiled) = compiled else {
        let rendered = render_all(&diags, &sdl, "schema.sdl", false);
        return Err(RunnerError::Compile { file: "schema.sdl (adopted)".into(), rendered, diagnostics: diags, source: sdl });
    };
    if compiled != ir {
        return Err(RunnerError::Project(
            "internal error: the adopted schema does not round-trip through SDL unchanged; nothing was written".into(),
        ));
    }

    let counts = Counts {
        tables: ir.tables.len(),
        columns: ir.tables.iter().map(|t| t.columns.len()).sum(),
        enums: ir.enums.len(),
        types: ir.types.len(),
        sequences: ir.sequences.len(),
        indexes: ir.tables.iter().map(|t| t.indexes.len()).sum(),
        constraints: ir.tables.iter().map(|t| t.constraints.len()).sum(),
    };
    Ok(Prepared { ir, sdl, omissions: om, counts })
}

/// Adopt the database behind `exec` into `project` (which must be fresh).
pub fn adopt(project: &Project, exec: &mut dyn Executor, opts: &AdoptOptions) -> Result<AdoptReport, RunnerError> {
    crate::runner::check_dialect(project, exec)?;
    // ---- preconditions: this is for a fresh project and an unmanaged database
    if !list(project)?.is_empty() || !is_empty_ir(&project.state_ir()?) {
        return Err(RunnerError::Project(
            "this project already has migrations; adopt is for a fresh project (run `certo sdl migrate init` in an empty directory)".into(),
        ));
    }
    let schema_path = project.schema_path();
    let previous = fs::read_to_string(&schema_path).unwrap_or_default();
    let (file, _) = parse(&previous);
    if !file.decls.is_empty() && !opts.force {
        return Err(RunnerError::Project(format!(
            "{} already contains a schema; adopt would overwrite it (use --force to replace it)",
            schema_path.display()
        )));
    }
    let applied = exec.applied().map_err(|e| RunnerError::Connection(e.message))?;
    if !applied.is_empty() {
        return Err(RunnerError::Project(format!(
            "the database already has a migration history ({} applied); adopt is for databases that are not managed yet",
            applied.len()
        )));
    }

    let live = exec.introspect().map_err(|e| RunnerError::Connection(e.message))?;
    let prepared = prepare_for(project.dialect(), live)?;
    if prepared.counts.tables == 0 && prepared.counts.enums == 0 && prepared.counts.types == 0 && prepared.counts.sequences == 0 {
        return Err(RunnerError::Project(
            "nothing to adopt: the database has no tables, enums or types SDL can express (for a new database, write schema.sdl and run `migrate new`)".into(),
        ));
    }

    if opts.dry_run {
        return Ok(AdoptReport {
            dry_run: true,
            schema_sdl: prepared.sdl,
            adopted: prepared.counts,
            omissions: prepared.omissions,
            migration: None,
            known_drift: Vec::new(),
        });
    }

    // ---- write schema.sdl, freeze the baseline, then record it as applied
    fs::write(&schema_path, &prepared.sdl).map_err(|e| io(&schema_path, e))?;
    let rollback = |project: &Project| {
        let _ = fs::write(project.schema_path(), &previous);
        let _ = project.write_state_ir(&SchemaIR::empty());
    };
    let created = match create(project, "baseline", None, false) {
        Ok(c) => c,
        Err(e) => {
            rollback(project);
            return Err(e);
        }
    };
    let baseline = list(project)?.into_iter().find(|m| m.seq == created.seq).expect("just created");

    // annotate the human-readable copy (never executed, not checksummed)
    let up_sql = created.dir.join("up.sql");
    if let Ok(text) = fs::read_to_string(&up_sql) {
        let note = "-- BASELINE: recorded as applied by `migrate adopt`; the database already had these objects.\n-- Running it on an EMPTY database builds them.\n";
        let _ = fs::write(&up_sql, format!("{note}{text}"));
    }

    if let Err(e) = exec.record_applied(&baseline) {
        let _ = fs::remove_dir_all(&created.dir); // don't leave a baseline the database does not know about
        rollback(project);
        return Err(RunnerError::Connection(format!("could not record the baseline migration: {}", e.message)));
    }

    let known_drift = drift::check(project, exec).map(|d| d.items).unwrap_or_default();
    Ok(AdoptReport {
        dry_run: false,
        schema_sdl: prepared.sdl,
        adopted: prepared.counts,
        omissions: prepared.omissions,
        migration: Some(baseline.label()),
        known_drift,
    })
}

fn is_empty_ir(ir: &SchemaIR) -> bool {
    ir.tables.is_empty() && ir.enums.is_empty() && ir.types.is_empty() && ir.sequences.is_empty()
}
