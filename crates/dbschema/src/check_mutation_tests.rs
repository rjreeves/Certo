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
fn insert_ok() {
    let src = format!(r#"
{ORDERS}
fn create(conn: Int): Int =
    Mutation.insertInto("Orders")
        |> Mutation.set("status", "pending")
        |> Mutation.set("total", "19.99")
        |> Mutation.run(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn update_ok() {
    let src = format!(r#"
{ORDERS}
fn ship(conn: Int, id: Text): Int =
    Mutation.updateTable("Orders")
        |> Mutation.set("status", "shipped")
        |> Mutation.filter("id", "=", id)
        |> Mutation.run(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn delete_ok() {
    let src = format!(r#"
{ORDERS}
fn cancel(conn: Int, id: Text): Int =
    Mutation.deleteFrom("Orders")
        |> Mutation.filter("id", "=", id)
        |> Mutation.run(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn upsert_ok() {
    let src = format!(r#"
{ORDERS}
fn upsertOrder(conn: Int): Int =
    Mutation.insertInto("Orders")
        |> Mutation.set("id", "x")
        |> Mutation.set("status", "pending")
        |> Mutation.onConflict("id")
        |> Mutation.run(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn insert_many_ok() {
    let src = format!(r#"
{ORDERS}
fn seed(conn: Int): Int =
    Mutation.insertMany("Orders", ["status", "total"])
        |> Mutation.addRow(["pending", "19.99"])
        |> Mutation.addRow(["paid", "42.00"])
        |> Mutation.run(conn)
"#);
    check(&src).unwrap();
}

#[test]
fn fully_applied_calls_ok() {
    let src = format!(r#"
{ORDERS}
fn ship(conn: Int, id: Text): Int = {{
    val m  = Mutation.updateTable("Orders")
    val m2 = Mutation.set(m, "status", "shipped")
    val m3 = Mutation.filter(m2, "id", "=", id)
    Mutation.run(m3, conn)
}}
"#);
    check(&src).unwrap();
}

// ------------------------------------------------------------------ //
// Errors
// ------------------------------------------------------------------ //

#[test]
fn unknown_table_is_rejected() {
    let src = r#"
module Test

fn create(conn: Int): Int =
    Mutation.insertInto("Ghost") |> Mutation.set("x", "1") |> Mutation.run(conn)
"#;
    let errs = check(src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationUnknownTable { table, .. } if table == "Ghost")),
        "expected E0519: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unknown_column_in_set_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn create(conn: Int): Int =
    Mutation.insertInto("Orders") |> Mutation.set("ghost_col", "1") |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownColumn { column, .. } if column == "ghost_col")),
        "expected E0509: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn filter_on_insert_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn create(conn: Int): Int =
    Mutation.insertInto("Orders")
        |> Mutation.set("status", "pending")
        |> Mutation.filter("id", "=", "x")
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationInvalidStage { function, .. } if function == "Mutation.filter")),
        "expected E0520: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn set_on_delete_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn cancel(conn: Int): Int =
    Mutation.deleteFrom("Orders")
        |> Mutation.set("status", "cancelled")
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationInvalidStage { function, .. } if function == "Mutation.set")),
        "expected E0520: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn on_conflict_on_update_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn ship(conn: Int): Int =
    Mutation.updateTable("Orders")
        |> Mutation.set("status", "shipped")
        |> Mutation.onConflict("id")
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationInvalidStage { function, .. } if function == "Mutation.onConflict")),
        "expected E0520: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn add_row_on_insert_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn create(conn: Int): Int =
    Mutation.insertInto("Orders")
        |> Mutation.addRow(["pending", "19.99"])
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationInvalidStage { function, .. } if function == "Mutation.addRow")),
        "expected E0520: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn insert_many_unknown_column_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn seed(conn: Int): Int =
    Mutation.insertMany("Orders", ["ghost_col", "total"])
        |> Mutation.addRow(["x", "1"])
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryUnknownColumn { column, .. } if column == "ghost_col")),
        "expected E0509: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn add_row_arity_mismatch_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn seed(conn: Int): Int =
    Mutation.insertMany("Orders", ["status", "total"])
        |> Mutation.addRow(["pending"])
        |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MutationRowArityMismatch { expected: 2, found: 1 })),
        "expected E0521: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn non_literal_column_in_set_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn create(conn: Int, col: Text): Int =
    Mutation.insertInto("Orders") |> Mutation.set(col, "1") |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryNonLiteralArg { function, position }
        if function == "Mutation.set" && position == "column")),
        "expected E0510: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn invalid_operator_in_filter_is_rejected() {
    let src = format!(r#"
{ORDERS}
fn cancel(conn: Int, id: Text): Int =
    Mutation.deleteFrom("Orders") |> Mutation.filter("id", "~=", id) |> Mutation.run(conn)
"#);
    let errs = check(&src).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::QueryInvalidOperator { op } if op == "~=")),
        "expected E0511: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}
