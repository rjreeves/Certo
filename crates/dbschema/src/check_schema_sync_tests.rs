use crate::{check_schema_sync, DbErrorKind, LiveColumn, LiveTable, Schema};

fn parse(src: &str) -> certo_ast::module::Module {
    certo_parser::parse(src).expect("source must parse")
}

fn has(errs: &[crate::DbError], f: impl Fn(&DbErrorKind) -> bool) -> bool {
    errs.iter().any(|e| f(&e.kind))
}

fn live_orders(columns: Vec<(&str, &str, bool)>) -> Vec<LiveTable> {
    vec![LiveTable {
        name: "orders".to_string(),
        columns: columns.into_iter().map(|(name, certo_type, nullable)| LiveColumn {
            name: name.to_string(), certo_type: certo_type.to_string(), nullable,
        }).collect(),
    }]
}

const ORDERS_SRC: &str = r#"
module Test

type Orders = {
    id: UUID
    status: Text
    total: Decimal
}

impl DbRow for Orders {}
"#;

#[test]
fn matching_schema_is_ok() {
    let module = parse(ORDERS_SRC);
    let schema = Schema::build(&module);
    let live = live_orders(vec![
        ("id", "UUID", false),
        ("status", "Text", false),
        ("total", "Decimal", false),
    ]);
    let errs = check_schema_sync(&module, &schema, &live);
    assert!(errs.is_empty(), "expected no errors: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn snake_case_and_camel_case_names_are_bridged() {
    // Live table "orders" / column "customer_id" should match `type Orders` / field `customerId`.
    let module = parse(r#"
module Test

type Orders = {
    customerId: UUID
}

impl DbRow for Orders {}
"#);
    let schema = Schema::build(&module);
    let live = live_orders(vec![("customer_id", "UUID", false)]);
    let errs = check_schema_sync(&module, &schema, &live);
    assert!(errs.is_empty(), "expected no errors: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn missing_table_is_rejected() {
    let module = parse(ORDERS_SRC);
    let schema = Schema::build(&module);
    let errs = check_schema_sync(&module, &schema, &[]); // no live tables at all
    assert!(has(&errs, |k| matches!(k, DbErrorKind::SchemaSyncTableMissing { table } if table == "Orders")),
        "expected E0522: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn missing_column_is_rejected() {
    let module = parse(ORDERS_SRC);
    let schema = Schema::build(&module);
    let live = live_orders(vec![("id", "UUID", false), ("status", "Text", false)]); // no "total"
    let errs = check_schema_sync(&module, &schema, &live);
    assert!(has(&errs, |k| matches!(k, DbErrorKind::SchemaSyncColumnMissing { column, .. } if column == "total")),
        "expected E0523: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn type_mismatch_is_rejected() {
    let module = parse(ORDERS_SRC);
    let schema = Schema::build(&module);
    let live = live_orders(vec![
        ("id", "UUID", false),
        ("status", "Text", false),
        ("total", "Int", false), // live column is Int, Certo declares Decimal
    ]);
    let errs = check_schema_sync(&module, &schema, &live);
    assert!(has(&errs, |k| matches!(k, DbErrorKind::SchemaSyncTypeMismatch { column, declared, live, .. }
        if column == "total" && declared == "Decimal" && live == "Int")),
        "expected E0524: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn nullability_mismatch_is_rejected() {
    let module = parse(ORDERS_SRC);
    let schema = Schema::build(&module);
    let live = live_orders(vec![
        ("id", "UUID", false),
        ("status", "Text", true), // live column is nullable, Certo declares it required
        ("total", "Decimal", false),
    ]);
    let errs = check_schema_sync(&module, &schema, &live);
    assert!(has(&errs, |k| matches!(k, DbErrorKind::SchemaSyncNullabilityMismatch { column, declared_nullable: false, live_nullable: true, .. } if column == "status")),
        "expected E0525: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn types_without_dbrow_impl_are_ignored() {
    // Plain data record — never meant to correspond to a database table.
    let module = parse(r#"
module Test

type Address = {
    street: Text
    city: Text
}
"#);
    let schema = Schema::build(&module);
    let errs = check_schema_sync(&module, &schema, &[]); // no live tables, no DbRow impl either
    assert!(errs.is_empty(), "expected no errors for non-DbRow types: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}
