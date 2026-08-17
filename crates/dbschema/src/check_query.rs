use std::collections::HashMap;

use certo_ast::decl::Decl;
use certo_ast::expr::{Arg, Expr, ExpectMatcher, Lit, Stmt};
use certo_ast::module::Module;
use certo_ast::pattern::Pattern;
use certo_ast::span::{S, Span};

use crate::error::{DbError, DbErrorKind};
use crate::schema::Schema;

/// Shared with `check_mutation` — `Mutation.filter` uses the same operator set as
/// `Query.filter`.
pub(crate) const VALID_OPS: &[&str] = &["=", "!=", "<", "<=", ">", ">=", "like"];
const VALID_DIRS:     &[&str] = &["asc", "desc"];
const VALID_AGG_FNS:  &[&str] = &["count", "sum", "avg", "min", "max"];
/// Terminals that assume `SELECT *` (row queries) or a single scalar aggregate —
/// meaningless (or outright invalid SQL) once `.groupBy`/`.aggregate` has reshaped
/// the query. `.groupedList`/`.limit`/`.offset`/`.sql` are unaffected and stay usable.
const GROUPED_INCOMPATIBLE: &[&str] = &[
    "Query.list", "Query.first", "Query.count", "Query.sum", "Query.avg", "Query.min", "Query.max",
];

/// Everything known about a `Query` value at a given point in a function body:
/// which schema tables it draws from — as `(alias, table)` pairs, base table first then
/// joins in order — and whether `.groupBy`/`.aggregate` has already reshaped its `SELECT`
/// list. The alias defaults to the table name for plain `.from`/`.join`/`.leftJoin`, so the
/// common case behaves exactly as if there were no aliasing at all; `.fromAs`/`.joinAs`/
/// `.leftJoinAs` let it diverge, which is what makes self-joins (the same table twice, each
/// under a different alias) possible.
#[derive(Clone)]
struct QueryState {
    tables:     Vec<(String, String)>,
    aggregated: bool,
}

impl QueryState {
    fn new(table: String, alias: String) -> Self {
        QueryState { tables: vec![(alias, table)], aggregated: false }
    }
}

/// Validate every `Query` builder call site in the module against the schema.
///
/// `Query.from`/`.filter`/`.orderBy`/`.join`/`.groupBy`/`.aggregate`/... are ordinary
/// functions (see `certo_stdlib::dbquery`), so nothing stops a caller from passing a
/// computed `Text` value for a table/column/operator/direction/aggregate-function
/// argument. But doing so would make it impossible to verify the query against the
/// schema at compile time — the entire point of the feature — so this pass requires
/// those arguments to be string literals and rejects anything else (E0510).
///
/// This is an AST-level pass (like `check_migrations`) because pipe chains
/// (`a |> f(b)`) are only desugared into direct calls during HIR lowering;
/// at this stage `Expr::Pipe` is still a distinct node we can walk directly.
pub fn check_queries(module: &Module, schema: &Schema) -> Vec<DbError> {
    let mut errors = Vec::new();
    for sdecl in &module.decls {
        if let Decl::Fn(f) = &sdecl.node {
            if let Some(body) = &f.body {
                let mut scope: HashMap<String, QueryState> = HashMap::new();
                check_expr(&body.node, &mut scope, schema, &mut errors);
            }
        }
    }
    errors
}

/// Returns the query state (tables + aggregation shape) this expression's *value*
/// represents, if that can be determined statically. Recurses into every
/// sub-expression so nested queries (inside lambdas, record fields, etc.) are still
/// checked even when the outer expression itself isn't a query.
fn check_expr(
    expr:   &Expr,
    scope:  &mut HashMap<String, QueryState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> Option<QueryState> {
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
                Some(state) => Some(check_query_stage(&right.node, state, scope, schema, errors)),
                None => {
                    // Not a query chain — still check the right side for nested queries.
                    check_expr(&right.node, scope, schema, errors);
                    None
                }
            }
        }

        Expr::App { func, args, span } => {
            check_call(func, args, *span, scope, schema, errors)
        }

        Expr::Block { stmts, .. } => {
            for stmt in stmts {
                check_stmt(stmt, scope, schema, errors);
            }
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
                if let Some(g) = &arm.guard {
                    check_expr(&g.node, scope, schema, errors);
                }
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

        Expr::Transaction { body, .. }
        | Expr::Unsafe { body, .. } => {
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
    scope:  &mut HashMap<String, QueryState>,
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
            if let Some(state) = state {
                scope.insert(name.node.clone(), state);
            }
        }
        Stmt::Assign { value, .. } => {
            check_expr(&value.node, scope, schema, errors);
        }
        Stmt::Defer { body, .. } => {
            check_expr(&body.node, scope, schema, errors);
        }
        Stmt::Expr { expr, .. } => {
            check_expr(&expr.node, scope, schema, errors);
        }
    }
}

/// Check a fully-applied call (`Query.from(...)`, `Query.filter(q, ...)`, or any
/// other function call — falls through to generic recursion for non-query calls).
/// Returns the query's state if this call produces/threads a `Query`.
fn check_call(
    func:   &S<Expr>,
    args:   &[Arg],
    span:   Span,
    scope:  &mut HashMap<String, QueryState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> Option<QueryState> {
    let name = call_name(&func.node);

    // Always recurse into arguments for nested queries, regardless of what
    // this call turns out to be.
    for a in args { check_expr(&a.value.node, scope, schema, errors); }

    match name.as_deref() {
        Some("Query.from") if args.len() == 1 => {
            let table = string_lit_arg(args, 0);
            from_impl(table.clone(), table, "Query.from", span, schema, errors)
        }

        // `Query.fromAs("Table", "alias")` — needed to give the base table an alias other
        // than its own name, most importantly for a self-join's *first* occurrence of the
        // repeated table (see `join_impl` for the second).
        Some("Query.fromAs") if args.len() == 2 => {
            from_impl(string_lit_arg(args, 0), string_lit_arg(args, 1), "Query.fromAs", span, schema, errors)
        }

        // Fully-applied form: `Query.filter(q, "col", "op", "value")`.
        Some("Query.filter") if args.len() == 4 => {
            let state = check_expr(&args[0].value.node, scope, schema, errors);
            if let Some(state) = &state {
                check_filter_args(&args[1..], state, span, schema, errors);
            }
            state
        }

        // Fully-applied form: `Query.orderBy(q, "col", "dir")`.
        Some("Query.orderBy") if args.len() == 3 => {
            let state = check_expr(&args[0].value.node, scope, schema, errors);
            if let Some(state) = &state {
                check_order_by_args(&args[1..], state, span, schema, errors);
            }
            state
        }

        // Fully-applied form: `Query.join(q, "Table", "Base.col", "Table.col")`.
        Some("Query.join") | Some("Query.leftJoin") if args.len() == 4 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| join_stage(&args[1..], &state, span, schema, errors))
        }

        // Fully-applied form: `Query.joinAs(q, "Table", "alias", "e.col", "alias.col")` —
        // needed for a self-join, where the joined table is the same as one already in
        // scope and so must be given a distinct alias to be referenceable at all.
        Some("Query.joinAs") | Some("Query.leftJoinAs") if args.len() == 5 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| join_as_stage(&args[1..], &state, span, schema, errors))
        }

        // Fully-applied form: `Query.groupBy(q, "col")`.
        Some("Query.groupBy") if args.len() == 2 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| group_by_stage(&args[1..], &state, span, schema, errors))
        }

        // Fully-applied form: `Query.aggregate(q, "sum", "col", "alias")`.
        Some("Query.aggregate") if args.len() == 4 => {
            check_expr(&args[0].value.node, scope, schema, errors)
                .map(|state| aggregate_stage(&args[1..], &state, span, schema, errors))
        }

        // Fully-applied form: `Query.having(q, "sum", "col", "op", "value")`.
        Some("Query.having") if args.len() == 5 => {
            let state = check_expr(&args[0].value.node, scope, schema, errors);
            if let Some(state) = &state {
                having_stage(&args[1..], state, span, schema, errors);
            }
            state
        }

        Some(n) if GROUPED_INCOMPATIBLE.contains(&n) && !args.is_empty() => {
            let state = check_expr(&args[0].value.node, scope, schema, errors);
            if let Some(state) = &state {
                if state.aggregated {
                    errors.push(DbError {
                        kind: DbErrorKind::QueryGroupedTerminalMisuse { function: n.to_string() },
                        span,
                    });
                }
            }
            state
        }

        Some("Query.limit") | Some("Query.offset") | Some("Query.sql") | Some("Query.groupedList")
            if !args.is_empty() =>
        {
            check_expr(&args[0].value.node, scope, schema, errors)
        }

        _ => None,
    }
}

/// A single pipe stage applied to a query state already resolved from the left side,
/// e.g. the `filter(...)`/`orderBy(...)` in `q |> Query.filter(...)`. These are partial
/// calls missing the receiver, so the arg count is one less than the fully-applied form.
fn check_query_stage(
    stage:  &Expr,
    state:  &QueryState,
    scope:  &mut HashMap<String, QueryState>,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> QueryState {
    let Expr::App { func, args, span } = stage else {
        check_expr(stage, scope, schema, errors);
        return state.clone();
    };
    let name = call_name(&func.node);
    for a in args { check_expr(&a.value.node, scope, schema, errors); }

    match name.as_deref() {
        // `where` is a reserved keyword in Certo (used in `where`-clause trait bounds),
        // so the query builder's filter stage is named `Query.filter`, not `Query.where`.
        Some("Query.filter") if args.len() == 3 => {
            check_filter_args(args, state, *span, schema, errors);
            state.clone()
        }
        Some("Query.orderBy") if args.len() == 2 => {
            check_order_by_args(args, state, *span, schema, errors);
            state.clone()
        }
        Some("Query.join") | Some("Query.leftJoin") if args.len() == 3 =>
            join_stage(args, state, *span, schema, errors),
        Some("Query.joinAs") | Some("Query.leftJoinAs") if args.len() == 4 =>
            join_as_stage(args, state, *span, schema, errors),
        Some("Query.groupBy") if args.len() == 1 =>
            group_by_stage(args, state, *span, schema, errors),
        Some("Query.aggregate") if args.len() == 3 =>
            aggregate_stage(args, state, *span, schema, errors),
        Some("Query.having") if args.len() == 4 => {
            having_stage(args, state, *span, schema, errors);
            state.clone()
        }
        Some(n) if GROUPED_INCOMPATIBLE.contains(&n) => {
            if state.aggregated {
                errors.push(DbError {
                    kind: DbErrorKind::QueryGroupedTerminalMisuse { function: n.to_string() },
                    span: *span,
                });
            }
            state.clone()
        }
        _ => state.clone(),
    }
}

/// `Query.from`/`Query.fromAs` — the table literal must be a declared schema type
/// (E0508); the alias (the table name itself, for plain `.from`) must be a valid
/// identifier (E0514).
fn from_impl(
    table:    Option<String>,
    alias:    Option<String>,
    function: &str,
    span:     Span,
    schema:   &Schema,
    errors:   &mut Vec<DbError>,
) -> Option<QueryState> {
    let table = match table {
        Some(t) => {
            if !schema.has_table(&t) {
                errors.push(DbError { kind: DbErrorKind::QueryUnknownTable { table: t.clone() }, span });
            }
            t
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: function.to_string(), position: "table".to_string() },
                span,
            });
            return None;
        }
    };
    let alias = match alias {
        Some(a) => {
            if !is_valid_identifier(&a) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidAlias { alias: a.clone() }, span });
            }
            a
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: function.to_string(), position: "alias".to_string() },
                span,
            });
            return None;
        }
    };
    Some(QueryState::new(table, alias))
}

/// `Query.join`/`Query.leftJoin` — alias defaults to the table literal, so joining the
/// same table a second time this way collides with itself (E0526), which is exactly the
/// signal that a self-join needs `.joinAs`/`.leftJoinAs` with a distinct alias instead.
fn join_stage(args: &[Arg], state: &QueryState, span: Span, schema: &Schema, errors: &mut Vec<DbError>) -> QueryState {
    let table = string_lit_arg(args, 0);
    join_impl(table.clone(), table, string_lit_arg(args, 1), string_lit_arg(args, 2), state, span, schema, errors)
}

/// `Query.joinAs`/`Query.leftJoinAs` — table and alias given separately, so the same
/// table can appear more than once in a query (a self-join) as long as each occurrence
/// gets a distinct alias.
fn join_as_stage(args: &[Arg], state: &QueryState, span: Span, schema: &Schema, errors: &mut Vec<DbError>) -> QueryState {
    join_impl(
        string_lit_arg(args, 0), string_lit_arg(args, 1),
        string_lit_arg(args, 2), string_lit_arg(args, 3),
        state, span, schema, errors,
    )
}

fn join_impl(
    table:     Option<String>,
    alias:     Option<String>,
    left_col:  Option<String>,
    right_col: Option<String>,
    state:     &QueryState,
    span:      Span,
    schema:    &Schema,
    errors:    &mut Vec<DbError>,
) -> QueryState {
    let mut new_state = state.clone();

    let table = match table {
        Some(t) => {
            if !schema.has_table(&t) {
                errors.push(DbError { kind: DbErrorKind::QueryUnknownTable { table: t.clone() }, span });
            }
            Some(t)
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: "join".to_string(), position: "table".to_string() },
                span,
            });
            None
        }
    };

    let alias = match alias {
        Some(a) => {
            if !is_valid_identifier(&a) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidAlias { alias: a.clone() }, span });
            }
            if new_state.tables.iter().any(|(existing, _)| existing == &a) {
                errors.push(DbError { kind: DbErrorKind::QueryDuplicateAlias { alias: a.clone() }, span });
            }
            Some(a)
        }
        None => {
            errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: "join".to_string(), position: "alias".to_string() },
                span,
            });
            None
        }
    };

    if let (Some(table), Some(alias)) = (&table, &alias) {
        new_state.tables.push((alias.clone(), table.clone()));
    }

    // Both ON-clause columns must be qualified (`alias.column`) — unlike `.filter`/
    // `.orderBy`, a bare name here is never allowed, since the whole point of an ON
    // clause is relating two specific (possibly identically-named) tables.
    for (col, position) in [(left_col, "left column"), (right_col, "right column")] {
        match col {
            Some(column) => {
                if !column.contains('.') {
                    errors.push(DbError { kind: DbErrorKind::QueryJoinColumnNotQualified { column }, span });
                } else {
                    push_column_error(check_column(&new_state.tables, &column, schema), span, errors);
                }
            }
            None => errors.push(DbError {
                kind: DbErrorKind::QueryNonLiteralArg { function: "join".to_string(), position: position.to_string() },
                span,
            }),
        }
    }

    new_state
}

fn group_by_stage(
    args:   &[Arg],
    state:  &QueryState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> QueryState {
    match string_lit_arg(args, 0) {
        Some(column) => { push_column_error(check_column(&state.tables, &column, schema), span, errors); }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "groupBy".to_string(), position: "column".to_string() },
            span,
        }),
    }
    let mut new_state = state.clone();
    new_state.aggregated = true;
    new_state
}

fn aggregate_stage(
    args:   &[Arg],
    state:  &QueryState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) -> QueryState {
    check_agg_fn_and_column(args, 0, 1, "aggregate", state, span, schema, errors);

    match string_lit_arg(args, 2) {
        Some(alias) => {
            if !is_valid_identifier(&alias) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidAlias { alias }, span });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "aggregate".to_string(), position: "alias".to_string() },
            span,
        }),
    }

    let mut new_state = state.clone();
    new_state.aggregated = true;
    new_state
}

fn having_stage(
    args:   &[Arg],
    state:  &QueryState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) {
    check_agg_fn_and_column(args, 0, 1, "having", state, span, schema, errors);

    match string_lit_arg(args, 2) {
        Some(op) => {
            if !VALID_OPS.contains(&op.as_str()) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidOperator { op }, span });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: "having".to_string(), position: "operator".to_string() },
            span,
        }),
    }
    // args[3] (the bound value) is never literal-checked — like `.filter`'s value,
    // it always travels as a `$N` parameter regardless of where it comes from.
}

/// Shared by `.aggregate`/`.having`: validates the aggregate-function literal and its
/// column literal (`"*"` is only valid for `count`).
fn check_agg_fn_and_column(
    args:      &[Arg],
    fn_idx:    usize,
    col_idx:   usize,
    function:  &str,
    state:     &QueryState,
    span:      Span,
    schema:    &Schema,
    errors:    &mut Vec<DbError>,
) {
    let agg = string_lit_arg(args, fn_idx);
    match &agg {
        Some(a) if !VALID_AGG_FNS.contains(&a.as_str()) => {
            errors.push(DbError { kind: DbErrorKind::QueryInvalidAggFn { agg: a.clone() }, span });
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: function.to_string(), position: "aggregate function".to_string() },
            span,
        }),
        _ => {}
    }

    match string_lit_arg(args, col_idx) {
        Some(column) => {
            let is_count_star = column == "*" && agg.as_deref() == Some("count");
            if !is_count_star {
                push_column_error(check_column(&state.tables, &column, schema), span, errors);
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg { function: function.to_string(), position: "column".to_string() },
            span,
        }),
    }
}

fn check_filter_args(
    args:   &[Arg],
    state:  &QueryState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) {
    match string_lit_arg(args, 0) {
        Some(column) => { push_column_error(check_column(&state.tables, &column, schema), span, errors); }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg {
                function: "filter".to_string(), position: "column".to_string(),
            },
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
            kind: DbErrorKind::QueryNonLiteralArg {
                function: "filter".to_string(), position: "operator".to_string(),
            },
            span,
        }),
    }
}

fn check_order_by_args(
    args:   &[Arg],
    state:  &QueryState,
    span:   Span,
    schema: &Schema,
    errors: &mut Vec<DbError>,
) {
    match string_lit_arg(args, 0) {
        Some(column) => { push_column_error(check_column(&state.tables, &column, schema), span, errors); }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg {
                function: "orderBy".to_string(), position: "column".to_string(),
            },
            span,
        }),
    }

    match string_lit_arg(args, 1) {
        Some(dir) => {
            if !VALID_DIRS.contains(&dir.to_ascii_lowercase().as_str()) {
                errors.push(DbError { kind: DbErrorKind::QueryInvalidSortDir { dir }, span });
            }
        }
        None => errors.push(DbError {
            kind: DbErrorKind::QueryNonLiteralArg {
                function: "orderBy".to_string(), position: "direction".to_string(),
            },
            span,
        }),
    }
}

/// The result of resolving a `.filter`/`.orderBy`/`.groupBy`/`.aggregate` column literal
/// (either bare, e.g. `"total"`, or qualified, e.g. `"Orders.total"`/`"e.managerId"`)
/// against the set of `(alias, table)` pairs currently in scope for a query (base table
/// plus any joins).
enum ColumnCheck {
    Ok,
    /// Bare column not found on any table in scope (or qualified column not found on its table).
    UnknownColumn(String, String),
    /// Qualified column's alias isn't the base table or a joined table's alias.
    TableNotJoined(String),
    /// Bare column matches more than one aliased table in scope — must be qualified.
    /// Lists the *aliases* it's ambiguous between (not table names — for a self-join both
    /// sides are the same table, so the alias is the only thing that disambiguates).
    Ambiguous(String, Vec<String>),
}

fn check_column(tables: &[(String, String)], column: &str, schema: &Schema) -> ColumnCheck {
    if let Some((alias, col)) = column.split_once('.') {
        match tables.iter().find(|(a, _)| a == alias) {
            None => ColumnCheck::TableNotJoined(alias.to_string()),
            Some((_, table)) => {
                if schema.column_type(table, col).is_none() {
                    ColumnCheck::UnknownColumn(table.clone(), col.to_string())
                } else {
                    ColumnCheck::Ok
                }
            }
        }
    } else {
        let matches: Vec<&(String, String)> = tables.iter()
            .filter(|(_, table)| schema.column_type(table, column).is_some())
            .collect();
        match matches.len() {
            0 => ColumnCheck::UnknownColumn(tables.first().map(|(_, t)| t.clone()).unwrap_or_default(), column.to_string()),
            1 => ColumnCheck::Ok,
            _ => ColumnCheck::Ambiguous(column.to_string(), matches.iter().map(|(a, _)| a.clone()).collect()),
        }
    }
}

fn push_column_error(check: ColumnCheck, span: Span, errors: &mut Vec<DbError>) {
    match check {
        ColumnCheck::Ok => {}
        ColumnCheck::UnknownColumn(table, column) =>
            errors.push(DbError { kind: DbErrorKind::QueryUnknownColumn { table, column }, span }),
        ColumnCheck::TableNotJoined(table) =>
            errors.push(DbError { kind: DbErrorKind::QueryColumnTableNotJoined { table }, span }),
        ColumnCheck::Ambiguous(column, tables) =>
            errors.push(DbError { kind: DbErrorKind::QueryAmbiguousColumn { column, tables }, span }),
    }
}

fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Shared with `check_mutation`.
pub(crate) fn string_lit_arg(args: &[Arg], idx: usize) -> Option<String> {
    match args.get(idx).map(|a| &a.value.node) {
        Some(Expr::Lit { value: Lit::String(s), .. }) => Some(s.clone()),
        _ => None,
    }
}

/// The dotted call name, e.g. `"Query.from"`. `TypeName.method(...)` call syntax parses
/// as field access on a path (`Expr::Field { expr: Expr::Path("Query"), field: "from" }`),
/// not as a single qualified `Expr::Path` — the parser can't tell "static method call" from
/// "field access on a value" apart syntactically, so we flatten the common single-level
/// case (`Ident.ident(...)`) back into a dotted name here.
/// Shared with `check_mutation`.
pub(crate) fn call_name(func: &Expr) -> Option<String> {
    match func {
        Expr::Path { path, .. } => Some(
            path.segments.iter().map(|s| s.node.as_str()).collect::<Vec<_>>().join(".")
        ),
        Expr::Field { expr, field, .. } => {
            if let Expr::Path { path, .. } = &expr.node {
                if path.segments.len() == 1 {
                    return Some(format!("{}.{}", path.segments[0].node, field.node));
                }
            }
            None
        }
        _ => None,
    }
}
