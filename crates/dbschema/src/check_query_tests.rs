use crate::{check_module, DbErrorKind};

fn check(src: &str) -> Result<crate::Schema, Vec<crate::DbError>> {
    let module = certo_parser::parse(src).expect("source must parse");
    check_module(&module)
}

fn has(errs: &[crate::DbError], f: impl Fn(&DbErrorKind) -> bool) -> bool {
    errs.iter().any(|e| f(&e.kind))
}

const ORDERS: &str = r#"
module Test

type Orders = {
    id: UUID
    status: Text
    total: Decimal
}
"#;

#[test]
fn piped_query_against_known_columns_ok() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): List<Orders> = {{
    Query.from("Orders")
        |> Query.filter("status", "=", "pending")
        |> Query.orderBy("total", "desc")
        |> Query.limit(20)
        |> Query.list(conn, OrdersFromRow)
}}

fn OrdersFromRow(row: List<Text?>): Orders = Orders {{ id: uuid"00000000-0000-0000-0000-000000000000", status: "x", total: d"0" }}
"#);
    check(&src).unwrap();
}

#[test]
fn fully_applied_query_calls_ok() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int = {{
    val q = Query.from("Orders")
    val q2 = Query.filter(q, "status", "=", "pending")
    val q3 = Query.orderBy(q2, "total", "asc")
    Query.count(q3, conn)
}}
"#);
    check(&src).unwrap();
}

#[test]
fn unknown_table_is_rejected() {
    let src = r#"
module Test

fn recent(conn: Int): Int =
    Query.from("Ghost") |> Query.count(conn)
"#;
    let errs = check(src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownTable { table } if table == "Ghost")),
        "expected E0508: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unknown_column_in_where_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int =
    Query.from("Orders") |> Query.filter("ghost_col", "=", "x") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownColumn { column, .. } if column == "ghost_col")),
        "expected E0509: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unknown_column_in_order_by_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int =
    Query.from("Orders") |> Query.orderBy("ghost_col", "asc") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownColumn { column, .. } if column == "ghost_col")),
        "expected E0509: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn invalid_operator_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int =
    Query.from("Orders") |> Query.filter("status", "~=", "x") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidOperator { op } if op == "~=")),
        "expected E0511: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn invalid_sort_dir_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int =
    Query.from("Orders") |> Query.orderBy("status", "sideways") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidSortDir { dir } if dir == "sideways")),
        "expected E0512: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn non_literal_column_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int, col: Text): Int =
    Query.from("Orders") |> Query.filter(col, "=", "x") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryNonLiteralArg { function, position }
        if function == "filter" && position == "column")),
        "expected E0510: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Joins
// ------------------------------------------------------------------ //

const ORDERS_AND_CUSTOMERS: &str = r#"
module Test

type Orders = {
    id: UUID
    status: Text
    total: Decimal
    customerId: UUID
}

type Customers = {
    id: UUID
    name: Text
}
"#;

#[test]
fn join_with_qualified_columns_ok() {
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.join("Customers", "Orders.customerId", "Customers.id")
        |> Query.filter("Customers.name", "=", "Alice")
        |> Query.count(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn left_join_ok() {
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.leftJoin("Customers", "Orders.customerId", "Customers.id")
        |> Query.count(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn join_unknown_table_is_rejected() {
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.join("Ghost", "Orders.customerId", "Ghost.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownTable { table } if table == "Ghost")),
        "expected E0508: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn join_column_not_qualified_is_rejected() {
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.join("Customers", "customerId", "Customers.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryJoinColumnNotQualified { column } if column == "customerId")),
        "expected E0518: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn join_column_table_not_in_query_is_rejected() {
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.join("Customers", "Widgets.customerId", "Customers.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryColumnTableNotJoined { table } if table == "Widgets")),
        "expected E0517: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn ambiguous_bare_column_after_join_is_rejected() {
    // Both Orders and Customers have an "id" column — a bare "id" filter is ambiguous.
    let src = format!(r#"
{ORDERS_AND_CUSTOMERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.join("Customers", "Orders.customerId", "Customers.id")
        |> Query.filter("id", "=", "x")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryAmbiguousColumn { column, .. } if column == "id")),
        "expected E0515: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Self-joins
// ------------------------------------------------------------------ //

const EMPLOYEES: &str = r#"
module Test

type Employees = {
    id: UUID
    name: Text
    managerId: UUID
}
"#;

#[test]
fn self_join_with_distinct_aliases_ok() {
    let src = format!(r#"
{EMPLOYEES}
fn withManagers(conn: Int): Int =
    Query.fromAs("Employees", "e")
        |> Query.leftJoinAs("Employees", "m", "e.managerId", "m.id")
        |> Query.filter("e.name", "=", "Alice")
        |> Query.orderBy("m.name", "asc")
        |> Query.count(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn self_inner_join_with_distinct_aliases_ok() {
    let src = format!(r#"
{EMPLOYEES}
fn withManagers(conn: Int): Int =
    Query.fromAs("Employees", "e")
        |> Query.joinAs("Employees", "m", "e.managerId", "m.id")
        |> Query.count(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn fully_applied_self_join_ok() {
    let src = format!(r#"
{EMPLOYEES}
fn withManagers(conn: Int): Int = {{
    val e = Query.fromAs("Employees", "e")
    val em = Query.joinAs(e, "Employees", "m", "e.managerId", "m.id")
    Query.count(em, conn)
}}
"#);
    check(&src).unwrap();
}

#[test]
fn plain_join_of_same_table_twice_is_rejected() {
    // Without an explicit alias, joining "Employees" to itself collides — the alias
    // defaults to the table name both times, so this is the signal to use `.joinAs`.
    let src = format!(r#"
{EMPLOYEES}
fn broken(conn: Int): Int =
    Query.from("Employees")
        |> Query.join("Employees", "Employees.managerId", "Employees.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryDuplicateAlias { alias } if alias == "Employees")),
        "expected E0526: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn duplicate_explicit_alias_is_rejected() {
    let src = format!(r#"
{EMPLOYEES}
fn broken(conn: Int): Int =
    Query.fromAs("Employees", "e")
        |> Query.joinAs("Employees", "e", "e.managerId", "e.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryDuplicateAlias { alias } if alias == "e")),
        "expected E0526: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn bare_column_ambiguous_across_self_join_is_rejected() {
    let src = format!(r#"
{EMPLOYEES}
fn broken(conn: Int): Int =
    Query.fromAs("Employees", "e")
        |> Query.leftJoinAs("Employees", "m", "e.managerId", "m.id")
        |> Query.filter("name", "=", "Alice")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryAmbiguousColumn { column, .. } if column == "name")),
        "expected E0515: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn join_alias_qualifier_unknown_alias_is_rejected() {
    let src = format!(r#"
{EMPLOYEES}
fn broken(conn: Int): Int =
    Query.fromAs("Employees", "e")
        |> Query.leftJoinAs("Employees", "m", "x.managerId", "m.id")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryColumnTableNotJoined { table } if table == "x")),
        "expected E0517: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn from_as_invalid_alias_is_rejected() {
    let src = format!(r#"
{EMPLOYEES}
fn broken(conn: Int): Int =
    Query.fromAs("Employees", "not valid!") |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidAlias { alias } if alias == "not valid!")),
        "expected E0514: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ------------------------------------------------------------------ //
// Aggregation
// ------------------------------------------------------------------ //

#[test]
fn grouped_aggregate_query_ok() {
    let src = format!(r#"
{ORDERS}
fn summary(conn: Int): List<Int> =
    Query.from("Orders")
        |> Query.groupBy("status")
        |> Query.aggregate("count", "*", "orderCount")
        |> Query.aggregate("sum", "total", "totalRevenue")
        |> Query.having("count", "*", ">", "0")
        |> Query.groupedList(conn, SummaryFromRow)

fn SummaryFromRow(row: List<Text?>): Int = 0
"#);
    check(&src).unwrap();
}

#[test]
fn ungrouped_scalar_aggregates_ok() {
    let src = format!(r#"
{ORDERS}
fn totals(conn: Int): Text? =
    Query.from("Orders")
        |> Query.filter("status", "=", "paid")
        |> Query.sum("total", conn)
"#);
    check(&src).unwrap();
}

#[test]
fn groupby_unknown_column_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn summary(conn: Int): List<Int> =
    Query.from("Orders") |> Query.groupBy("ghost_col") |> Query.groupedList(conn, SummaryFromRow)

fn SummaryFromRow(row: List<Text?>): Int = 0
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownColumn { column, .. } if column == "ghost_col")),
        "expected E0509: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn aggregate_invalid_agg_fn_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn summary(conn: Int): List<Int> =
    Query.from("Orders")
        |> Query.groupBy("status")
        |> Query.aggregate("median", "total", "med")
        |> Query.groupedList(conn, SummaryFromRow)

fn SummaryFromRow(row: List<Text?>): Int = 0
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidAggFn { agg } if agg == "median")),
        "expected E0513: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn aggregate_invalid_alias_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn summary(conn: Int): List<Int> =
    Query.from("Orders")
        |> Query.groupBy("status")
        |> Query.aggregate("sum", "total", "not valid!")
        |> Query.groupedList(conn, SummaryFromRow)

fn SummaryFromRow(row: List<Text?>): Int = 0
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidAlias { alias } if alias == "not valid!")),
        "expected E0514: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn having_invalid_operator_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn summary(conn: Int): List<Int> =
    Query.from("Orders")
        |> Query.groupBy("status")
        |> Query.aggregate("count", "*", "c")
        |> Query.having("count", "*", "~=", "0")
        |> Query.groupedList(conn, SummaryFromRow)

fn SummaryFromRow(row: List<Text?>): Int = 0
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidOperator { op } if op == "~=")),
        "expected E0511: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn list_after_groupby_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): List<Orders> =
    Query.from("Orders")
        |> Query.groupBy("status")
        |> Query.list(conn, OrdersFromRow)

fn OrdersFromRow(row: List<Text?>): Orders = Orders {{ id: uuid"00000000-0000-0000-0000-000000000000", status: "x", total: d"0" }}
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryGroupedTerminalMisuse { function } if function == "Query.list")),
        "expected E0516: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn count_after_aggregate_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn recent(conn: Int): Int =
    Query.from("Orders")
        |> Query.aggregate("sum", "total", "totalRevenue")
        |> Query.count(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryGroupedTerminalMisuse { function } if function == "Query.count")),
        "expected E0516: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}
