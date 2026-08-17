use std::collections::HashMap;

use certo_ast::decl::Decl;
use certo_ast::expr::{Arg, Expr, ExpectMatcher, Lit, Stmt};
use certo_ast::module::Module;
use certo_ast::pattern::Pattern;
use certo_ast::span::{S, Span};

use crate::check_query::{call_name, string_lit_arg, VALID_OPS};
use crate::error::{DbError, DbErrorKind};
use crate::schema::Schema;

#[derive(Clone, Copy, PartialEq)]
enum MutationKind { Insert, InsertMany, Update, Delete }

impl MutationKind {
    /// Human-readable description used in E0520 ("`.filter` cannot be used on {}").
    fn describe(self) -> &'static str {
        match self {
            MutationKind::Insert     => "an `insert` (use `.onConflict` for upsert semantics, not `.filter`)",
            MutationKind::InsertMany => "an `insertMany` (add rows with `.addRow`, not `.set`/`.filter`/`.onConflict`)",
            MutationKind::Update     => "an `update` (use `.set`, not `.onConflict`)",
            MutationKind::Delete     => "a `delete` (use `.filter`, not `.set`/`.onConflict`)",
        }
    }
}

/// Everything known about a `Mutation` value at a given point in a function body.
#[derive(Clone)]
struct MutationState {
    table: String,
    kind:  MutationKind,
    /// Column count declared at `.insertMany(table, columns)`, when `columns` was written
    /// as a literal list — used to catch `.addRow` arity mismatches (E0521).
    many_columns: Option<usize>,
}

/// Validate every `Mutation` builder call site in the module against the schema.
/// Mirrors `check_query::check_queries` — see that module's doc comment for why this is
/// an AST-level pass (pipe chains aren't desugared until HIR lowering).
pub fn check_mutations(module: &Module, schema: &Schema) -> Vec<DbError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            if let Some(body) = &f.body {
                let mut scope: HashMap<String, MutationState> = HashMap::new();
                check_expr(&body.node, &mut scope, schema, &mut errors);
            }
        }
    }
    errors
}

fn check_expr(
    expr:   &Expr,
    scope:  &mut HashMap<String, MutationState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> Option<MutationState> {
    match expr {
        Expr::Path { path, .. } => {
            if path.segments.len() == 1 {
                scope.get(path.segments[0].node.as_str()).cloned()
            } else {
                None
            }
        }

        Expr::Pipe { left, right, .. } => {
            let state = check_expr(&left.node, scope, schema, errors);
            match &state {
                Some(state) => Some(check_mutation_stage(&right.node, state, scope, schema, errors)),
                None => {
                    check_expr(&right.node, scope, schema, errors);
                    None
                }
            }
        }

        Expr::App { func, args, span } => check_call(func, args, *span, scope, schema, errors),

        Expr::Block { stmts, .. } => {
            for stmt in stmts { check_stmt(stmt, scope, schema, errors); }
            None
        }

        Expr::If { cond, then_expr, else_expr, .. } => {
            check_expr(&cond.node, scope, schema, errors);
            check_expr(&then_expr.node, scope, schema, errors);
            check_expr(&else_expr.node, scope, schema, errors);
            None
        }

        Expr::Match { scrutinee, arms, .. } => {
            check_expr(&scrutinee.node, scope, schema, errors);
            for arm in arms {
                if let Some(g) = &arm.guard { check_expr(&g.node, scope, schema, errors); }
                check_expr(&arm.body.node, scope, schema, errors);
            }
            None
        }

        Expr::Lambda { body, .. } => {
            check_expr(&body.node, scope, schema, errors);
            None
        }

        Expr::List { elements, .. } | Expr::Tuple { elements, .. } => {
            for e in elements { check_expr(&e.node, scope, schema, errors); }
            None
        }

        Expr::Record { base, fields, .. } => {
            if let Some(b) = base { check_expr(&b.node, scope, schema, errors); }
            for f in fields { check_expr(&f.value.node, scope, schema, errors); }
            None
        }

        Expr::BinOp { left, right, .. } => {
            check_expr(&left.node, scope, schema, errors);
            check_expr(&right.node, scope, schema, errors);
            None
        }

        Expr::UnOp { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Await { expr, .. }
        | Expr::Spawn { expr, .. }
        | Expr::Age { expr, .. } => {
            check_expr(&expr.node, scope, schema, errors);
            None
        }

        Expr::ExpectAssertion { actual, matcher, .. } => {
            check_expr(&actual.node, scope, schema, errors);
            if let ExpectMatcher::ToBe(y) = matcher { check_expr(&y.node, scope, schema, errors); }
            None
        }

        Expr::Field { expr, .. } | Expr::SafeField { expr, .. } => {
            check_expr(&expr.node, scope, schema, errors);
            None
        }

        Expr::Guard { cond, else_expr, .. } => {
            check_expr(&cond.node, scope, schema, errors);
            check_expr(&else_expr.node, scope, schema, errors);
            None
        }

        Expr::Require { expr, error, .. } => {
            check_expr(&expr.node, scope, schema, errors);
            check_expr(&error.node, scope, schema, errors);
            None
        }

        Expr::Parallel { tasks, timeout, .. } => {
            for t in tasks { check_expr(&t.node, scope, schema, errors); }
            if let Some(t) = timeout { check_expr(&t.node, scope, schema, errors); }
            None
        }

        Expr::WithTimeout { duration, body, .. } => {
            check_expr(&duration.node, scope, schema, errors);
            check_expr(&body.node, scope, schema, errors);
            None
        }

        Expr::Transaction { body, .. } | Expr::Unsafe { body, .. } => {
            check_expr(&body.node, scope, schema, errors);
            None
        }

        Expr::Ascribe { expr, .. } => {
            check_expr(&expr.node, scope, schema, errors);
            None
        }

        Expr::For { iter, body, .. } => {
            check_expr(&iter.node, scope, schema, errors);
            check_expr(&body.node, scope, schema, errors);
            None
        }

        Expr::While { cond, body, .. } => {
            check_expr(&cond.node, scope, schema, errors);
            check_expr(&body.node, scope, schema, errors);
            None
        }

        Expr::Lit { .. } => None,
    }
}

fn check_stmt(
    stmt:   &Stmt,
    scope:  &mut HashMap<String, MutationState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) {
    match stmt {
        Stmt::Val { pattern, value, .. } => {
            let state = check_expr(&value.node, scope, schema, errors);
            if let (Pattern::Ident { name, .. }, Some(state)) = (&pattern.node, state) {
                scope.insert(name.node.clone(), state);
            }
        }
        Stmt::Var { name, value, .. } => {
            let state = check_expr(&value.node, scope, schema, errors);
            if let Some(state) = state { scope.insert(name.node.clone(), state); }
        }
        Stmt::Assign { value, .. } => { check_expr(&value.node, scope, schema, errors); }
        Stmt::Defer { body, .. }   => { check_expr(&body.node, scope, schema, errors); }
        Stmt::Expr { expr, .. }    => { check_expr(&expr.node, scope, schema, errors); }
    }
}

fn check_call(
    func:   &S<Expr>,
    args:   &[Arg],
    span:   Span,
    scope:  &mut HashMap<String, MutationState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> Option<MutationState> {
    let name = call_name(&func.node);
    for a in args { check_expr(&a.value.node, scope, schema, errors); }

    match name.as_deref() {
        Some(f @ ("Mutation.insertInto" | "Mutation.updateTable" | "Mutation.deleteFrom")) if args.len() == 1 => {
            let kind = match f {
                "Mutation.insertInto" => MutationKind::Insert,
                "Mutation.updateTable" => MutationKind::Update,
                _ => MutationKind::Delete,
            };
            start_mutation(f, args, kind, span, schema, errors)
        }

        Some("Mutation.insertMany") if args.len() == 2 => {
            let mut state = start_mutation("Mutation.insertMany", &args[..1], MutationKind::InsertMany, span, schema, errors)?;
            let columns = check_insert_many_columns(&args[1..], &state, span, schema, errors);
            state.many_columns = columns;
            Some(state)
        }

        // Fully-applied forms: `Mutation.set(m, ...)`, `.filter(m, ...)`, `.onConflict(m, ...)`, `.addRow(m, ...)`.
        Some("Mutation.set") if args.len() == 3 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| { set_stage(&args[1..], &state, span, schema, errors); state })
        }
        Some("Mutation.filter") if args.len() == 4 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| { filter_stage(&args[1..], &state, span, schema, errors); state })
        }
        Some("Mutation.onConflict") if args.len() == 2 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| { on_conflict_stage(&args[1..], &state, span, schema, errors); state })
        }
        Some("Mutation.addRow") if args.len() == 2 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| { add_row_stage(&args[1..], &state, span, errors); state })
        }

        Some("Mutation.run") if !args.is_empty() => {
            check_expr(&args[0].value.node, scope, schema, errors)
        }

        _ => None,
    }
}

fn check_mutation_stage(
    stage:  &Expr,
    state:  &MutationState,
    scope:  &mut HashMap<String, MutationState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> MutationState {
    let Expr::App { func, args, span } = stage else {
        check_expr(stage, scope, schema, errors);
        return state.clone();
    };
    let name = call_name(&func.node);
    for a in args { check_expr(&a.value.node, scope, schema, errors); }

    match name.as_deref() {
        Some("Mutation.set") if args.len() == 2 => {
            set_stage(args, state, *span, schema, errors);
            state.clone()
        }
        Some("Mutation.filter") if args.len() == 3 => {
            filter_stage(args, state, *span, schema, errors);
            state.clone()
        }
        Some("Mutation.onConflict") if args.len() == 1 => {
            on_conflict_stage(args, state, *span, schema, errors);
            state.clone()
        }
        Some("Mutation.addRow") if args.len() == 1 => {
            add_row_stage(args, state, *span, errors);
            state.clone()
        }
        _ => state.clone(),
    }
}

fn start_mutation(
    function: &str,
    args:     &[Arg],
    kind:     MutationKind,
    span:     Span,
    schema:   &Schema,
    errors:   &mut Vec<DbError>,
) -> Option<MutationState> {
    match string_lit_arg(args, 0) {
        Some(table) => {
            if !schema.has_table(&table) {
                errors.push(DbError {
                    kind: DbErrorKind::MutationUnknownTable { function: function.to_string(), table: table.clone() },
                    span,
                });
            }
            Some(MutationState { table, kind, many_columns: None })
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: function.to_string(), position: "table".to_string() },
                span,
            });
            None
        }
    }
}

fn require_kind(function: &str, state: &MutationState, allowed: &[MutationKind], span: Span, errors: &mut Vec<DbError>) -> bool {
    if allowed.contains(&state.kind) { return true; }
    errors.push(DbError {
        kind: DbErrorKind::MutationInvalidStage {
            function: function.to_string(),
            kind: state.kind.describe().to_string(),
        },
        span,
    });
    false
}

fn set_stage(args: &[Arg], state: &MutationState, span: Span, schema: &Schema, errors: &mut Vec<DbError>) {
    if !require_kind("Mutation.set", state, &[MutationKind::Insert, MutationKind::Update], span, errors) { return; }
    match string_lit_arg(args, 0) {
        Some(column) => {
            if schema.column_type(&state.table, &column).is_none() {
                errors.push(DbError {
                    kind: DbErrorKind::QueryUnknownColumn { table: state.table.clone(), column },
                    span,
                });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "Mutation.set".to_string(), position: "column".to_string() },
            span,
        }),
    }
}

fn filter_stage(args: &[Arg], state: &MutationState, span: Span, schema: &Schema, errors: &mut Vec<DbError>) {
    if !require_kind("Mutation.filter", state, &[MutationKind::Update, MutationKind::Delete], span, errors) { return; }
    match string_lit_arg(args, 0) {
        Some(column) => {
            if schema.column_type(&state.table, &column).is_none() {
                errors.push(DbError {
                    kind: DbErrorKind::QueryUnknownColumn { table: state.table.clone(), column },
                    span,
                });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "Mutation.filter".to_string(), position: "column".to_string() },
            span,
        }),
    }
    match string_lit_arg(args, 1) {
        Some(op) => {
            if !VALID_OPS.contains(&op.as_str()) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidOperator { op }, span });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "Mutation.filter".to_string(), position: "operator".to_string() },
            span,
        }),
    }
}

fn on_conflict_stage(args: &[Arg], state: &MutationState, span: Span, schema: &Schema, errors: &mut Vec<DbError>) {
    if !require_kind("Mutation.onConflict", state, &[MutationKind::Insert], span, errors) { return; }
    match string_lit_arg(args, 0) {
        Some(column) => {
            if schema.column_type(&state.table, &column).is_none() {
                errors.push(DbError {
                    kind: DbErrorKind::QueryUnknownColumn { table: state.table.clone(), column },
                    span,
                });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "Mutation.onConflict".to_string(), position: "column".to_string() },
            span,
        }),
    }
}

fn add_row_stage(args: &[Arg], state: &MutationState, span: Span, errors: &mut Vec<DbError>) {
    if !require_kind("Mutation.addRow", state, &[MutationKind::InsertMany], span, errors) { return; }
    if let (Some(expected), Some(values)) = (state.many_columns, string_list_lit_arg(args, 0)) {
        if values.len() != expected {
            errors.push(DbError {
                kind: DbErrorKind::MutationRowArityMismatch { expected, found: values.len() },
                span,
            });
        }
    }
    // A non-literal `values` list (e.g. built from `List.map`) can't be arity-checked
    // statically — that's fine, it's just data; only column *names* need to be literal.
}

/// Validates `.insertMany(table, columns)`'s `columns` argument: must be a literal list of
/// string literals, each an existing column on `table`. Returns the column count when the
/// list is literal (used later to arity-check `.addRow`), or `None` otherwise.
fn check_insert_many_columns(
    args:   &[Arg],
    state:  &MutationState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> Option<usize> {
    match string_list_lit_arg(args, 0) {
        Some(columns) => {
            for column in &columns {
                if schema.column_type(&state.table, column).is_none() {
                    errors.push(DbError {
                        kind: DbErrorKind::QueryUnknownColumn { table: state.table.clone(), column: column.clone() },
                        span,
                    });
                }
            }
            Some(columns.len())
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: "Mutation.insertMany".to_string(), position: "columns".to_string() },
                span,
            });
            None
        }
    }
}

/// Extracts a literal `["a", "b", ...]` list of string literals, or `None` if the argument
/// isn't a list literal or contains a non-string-literal element.
fn string_list_lit_arg(args: &[Arg], idx: usize) -> Option<Vec<String>> {
    match args.get(idx).map(|a| &a.value.node) {
        Some(Expr::List { elements, .. }) => {
            elements.iter().map(|e| match &e.node {
                Expr::Lit { value: Lit::String(s), .. } => Some(s.clone()),
                _ => None,
            }).collect()
        }
        _ => None,
    }
}
