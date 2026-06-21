use certo_ast::decl::{
    Decl, TypeDecl, TypeBody, RecordTypeDef, RecordFieldDef,
    MigrationDecl, MigrationOp, ColumnDef, AlterOp, FkAction,
};
use certo_ast::module::Module;
use certo_ast::span::{S, Span};
use certo_ast::types::{TypeExpr, ModulePath};
use crate::{check_module, DbErrorKind};
use crate::schema::{Schema, SchemaColumn, SchemaTable, DriftItem};

const DUMMY: Span = Span::DUMMY;

fn ident(s: &str) -> S<String> { S::new(s.to_string(), DUMMY) }

fn named_ty(name: &str) -> S<TypeExpr> {
    S::new(TypeExpr::Named {
        path: ModulePath { segments: vec![ident(name)], span: DUMMY },
        args: vec![],
        span: DUMMY,
    }, DUMMY)
}

fn record_type(name: &str, fields: &[(&str, &str)]) -> Decl {
    Decl::Type(TypeDecl {
        is_pub:      false,
        name:        ident(name),
        type_params: vec![],
        body:        TypeBody::Record(RecordTypeDef {
            fields: fields.iter().map(|(n, t)| RecordFieldDef {
                name:     ident(n),
                ty:       named_ty(t),
                optional: false,
                span:     DUMMY,
            }).collect(),
            computed: vec![],
            span: DUMMY,
        }),
        span: DUMMY,
    })
}

fn col(name: &str, ty: &str) -> ColumnDef {
    ColumnDef {
        name:        name.to_string(),
        ty:          named_ty(ty),
        primary_key: false,
        nullable:    false,
        unique:      false,
        default:     None,
        span:        DUMMY,
    }
}

fn migration(name: &str, up: Vec<MigrationOp>, down: Vec<MigrationOp>) -> Decl {
    Decl::Migration(MigrationDecl {
        name:        name.to_string(),
        description: None,
        up,
        down,
        span: DUMMY,
    })
}

fn module(decls: Vec<Decl>) -> Module {
    Module {
        path:    ModulePath { segments: vec![ident("A")], span: DUMMY },
        imports: vec![],
        decls:   decls.into_iter().map(|d| S::new(d, DUMMY)).collect(),
        span:    DUMMY,
    }
}

fn has(errs: &[crate::DbError], f: impl Fn(&DbErrorKind) -> bool) -> bool {
    errs.iter().any(|e| f(&e.kind))
}

// ------------------------------------------------------------------ //
// Happy-path
// ------------------------------------------------------------------ //

#[test]
fn empty_module_ok() {
    check_module(&module(vec![])).unwrap();
}

#[test]
fn type_with_no_migration_ok() {
    let m = module(vec![record_type("User", &[("id", "UUID"), ("name", "Text")])]);
    check_module(&m).unwrap();
}

#[test]
fn valid_create_table_migration() {
    let m = module(vec![
        record_type("Order", &[("id", "UUID"), ("amount", "Decimal")]),
        migration("001_create_orders",
            vec![MigrationOp::CreateTable {
                name:    "Order".into(),
                columns: vec![col("id", "UUID"), col("amount", "Decimal")],
                span:    DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "Order".into(), span: DUMMY }],
        ),
    ]);
    check_module(&m).unwrap();
}

#[test]
fn create_index_after_create_table_ok() {
    let m = module(vec![
        record_type("Product", &[("id", "UUID"), ("sku", "Text")]),
        migration("002_create_products",
            vec![
                MigrationOp::CreateTable {
                    name:    "Product".into(),
                    columns: vec![col("id", "UUID"), col("sku", "Text")],
                    span:    DUMMY,
                },
                MigrationOp::CreateIndex {
                    name:    "idx_product_sku".into(),
                    table:   "Product".into(),
                    columns: vec!["sku".into()],
                    span:    DUMMY,
                },
            ],
            vec![MigrationOp::DropTable { name: "Product".into(), span: DUMMY }],
        ),
    ]);
    check_module(&m).unwrap();
}

#[test]
fn foreign_key_to_existing_table_ok() {
    let m = module(vec![
        record_type("User",  &[("id", "UUID")]),
        record_type("Post",  &[("id", "UUID"), ("user_id", "UUID")]),
        migration("003",
            vec![
                MigrationOp::CreateTable {
                    name: "User".into(), columns: vec![col("id", "UUID")], span: DUMMY,
                },
                MigrationOp::CreateTable {
                    name: "Post".into(), columns: vec![col("id", "UUID"), col("user_id", "UUID")], span: DUMMY,
                },
                MigrationOp::AlterTable {
                    name: "Post".into(),
                    ops:  vec![AlterOp::AddForeignKey {
                        column:     "user_id".into(),
                        references: "User".into(),
                        on_delete:  FkAction::Cascade,
                        span:       DUMMY,
                    }],
                    span: DUMMY,
                },
            ],
            vec![MigrationOp::DropTable { name: "Post".into(), span: DUMMY },
                 MigrationOp::DropTable { name: "User".into(), span: DUMMY }],
        ),
    ]);
    check_module(&m).unwrap();
}

// ------------------------------------------------------------------ //
// Error cases
// ------------------------------------------------------------------ //

#[test]
fn create_table_not_declared_as_type() {
    let m = module(vec![
        migration("001",
            vec![MigrationOp::CreateTable {
                name: "Ghost".into(), columns: vec![], span: DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "Ghost".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::TableNotDeclaredAsType { table, .. } if table == "Ghost")),
        "expected E0505: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn column_type_mismatch() {
    let m = module(vec![
        record_type("Item", &[("price", "Decimal")]),
        migration("001",
            vec![MigrationOp::CreateTable {
                name:    "Item".into(),
                columns: vec![col("price", "Int")],  // wrong type
                span:    DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "Item".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::ColumnTypeMismatch { column, .. } if column == "price")),
        "expected E0501: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn unknown_column_in_migration() {
    let m = module(vec![
        record_type("Widget", &[("id", "UUID")]),
        migration("001",
            vec![MigrationOp::CreateTable {
                name:    "Widget".into(),
                columns: vec![col("id", "UUID"), col("ghost_field", "Text")],
                span:    DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "Widget".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::UnknownColumn { column, .. } if column == "ghost_field")),
        "expected E0506: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn alter_table_not_created() {
    let m = module(vec![
        record_type("Thing", &[("id", "UUID"), ("name", "Text")]),
        migration("001",
            vec![MigrationOp::AlterTable {
                name: "Thing".into(),
                ops:  vec![AlterOp::AddColumn { def: col("name", "Text") }],
                span: DUMMY,
            }],
            vec![],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::TableNotCreated { table, .. } if table == "Thing")),
        "expected E0507: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn duplicate_migration_name() {
    let m = module(vec![
        record_type("Foo", &[("id", "UUID")]),
        migration("001",
            vec![MigrationOp::CreateTable { name: "Foo".into(), columns: vec![col("id", "UUID")], span: DUMMY }],
            vec![MigrationOp::DropTable { name: "Foo".into(), span: DUMMY }],
        ),
        migration("001",  // duplicate
            vec![MigrationOp::CreateTable { name: "Foo".into(), columns: vec![col("id", "UUID")], span: DUMMY }],
            vec![MigrationOp::DropTable { name: "Foo".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::DuplicateMigration { name, .. } if name == "001")),
        "expected E0503: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn missing_down_migration() {
    let m = module(vec![
        record_type("Bar", &[("id", "UUID")]),
        migration("001",
            vec![MigrationOp::CreateTable { name: "Bar".into(), columns: vec![col("id", "UUID")], span: DUMMY }],
            vec![],  // no down
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::MissingDownMigration { name, .. } if name == "001")),
        "expected E0504: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

#[test]
fn foreign_key_to_unknown_table() {
    let m = module(vec![
        record_type("Comment", &[("id", "UUID"), ("post_id", "UUID")]),
        migration("001",
            vec![
                MigrationOp::CreateTable {
                    name: "Comment".into(),
                    columns: vec![col("id", "UUID"), col("post_id", "UUID")],
                    span: DUMMY,
                },
                MigrationOp::AlterTable {
                    name: "Comment".into(),
                    ops: vec![AlterOp::AddForeignKey {
                        column:     "post_id".into(),
                        references: "Post".into(),   // Post doesn't exist
                        on_delete:  FkAction::Cascade,
                        span:       DUMMY,
                    }],
                    span: DUMMY,
                },
            ],
            vec![MigrationOp::DropTable { name: "Comment".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::UnknownForeignKeyTarget { references, .. } if references == "Post")),
        "expected E0502: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// ── Schema snapshot tests ────────────────────────────────────────────────────

fn make_schema(tables: &[(&str, &[(&str, &str, bool)])]) -> Schema {
    let mut schema = Schema { version: 1, ..Default::default() };
    for (table_name, cols) in tables {
        let columns = cols.iter().map(|(col, ty, nullable)| SchemaColumn {
            name:     col.to_string(),
            ty:       ty.to_string(),
            nullable: *nullable,
        }).collect();
        schema.tables.insert(table_name.to_string(), SchemaTable {
            name:    table_name.to_string(),
            columns,
        });
    }
    schema
}

#[test]
fn schema_save_load_roundtrip() {
    let original = make_schema(&[
        ("Order",  &[("id", "UUID", false), ("total", "Decimal", false)]),
        ("Customer", &[("id", "UUID", false), ("name", "Text", true)]),
    ]);
    let dir = std::env::temp_dir();
    let path = dir.join("certo_test_schema_roundtrip.json");
    original.save(&path).expect("save failed");
    let loaded = Schema::load(&path).expect("load failed");
    assert_eq!(original, loaded);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn schema_load_missing_file_returns_error() {
    let result = Schema::load(std::path::Path::new("/nonexistent/schema.json"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("cannot read"));
}

#[test]
fn schema_diff_no_drift() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false)])]);
    assert!(snap.diff(&live).is_empty());
}

#[test]
fn schema_diff_missing_table() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false)]), ("Invoice", &[("id", "UUID", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d, DriftItem::TableMissing { table } if table == "Invoice")),
        "expected TableMissing(Invoice): {:?}", drift);
}

#[test]
fn schema_diff_extra_table() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false)]), ("Audit", &[("id", "UUID", false)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d, DriftItem::TableExtra { table } if table == "Audit")),
        "expected TableExtra(Audit): {:?}", drift);
}

#[test]
fn schema_diff_missing_column() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false), ("total", "Decimal", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d,
        DriftItem::ColumnMissing { table, column } if table == "Order" && column == "total")),
        "expected ColumnMissing(Order.total): {:?}", drift);
}

#[test]
fn schema_diff_extra_column() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false), ("note", "Text", true)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d,
        DriftItem::ColumnExtra { table, column } if table == "Order" && column == "note")),
        "expected ColumnExtra(Order.note): {:?}", drift);
}

#[test]
fn schema_diff_type_drift() {
    let snap = make_schema(&[("Order", &[("id", "UUID", false), ("amount", "Int", false)])]);
    let live = make_schema(&[("Order", &[("id", "UUID", false), ("amount", "Decimal", false)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d,
        DriftItem::ColumnTypeDrift { table, column, snapshot, live }
        if table == "Order" && column == "amount" && snapshot == "Int" && live == "Decimal")),
        "expected ColumnTypeDrift: {:?}", drift);
}

#[test]
fn schema_diff_nullability_drift() {
    let snap = make_schema(&[("Order", &[("note", "Text", false)])]);
    let live = make_schema(&[("Order", &[("note", "Text", true)])]);
    let drift = snap.diff(&live);
    assert!(drift.iter().any(|d| matches!(d,
        DriftItem::NullabilityDrift { table, column, snapshot, live }
        if table == "Order" && column == "note" && !snapshot && *live)),
        "expected NullabilityDrift: {:?}", drift);
}

#[test]
fn drift_item_display() {
    assert_eq!(
        DriftItem::TableMissing { table: "Order".into() }.to_string(),
        "MISSING  table  Order"
    );
    assert_eq!(
        DriftItem::TableExtra { table: "Audit".into() }.to_string(),
        "EXTRA    table  Audit"
    );
    assert_eq!(
        DriftItem::ColumnMissing { table: "Order".into(), column: "total".into() }.to_string(),
        "MISSING  column Order.total"
    );
    assert_eq!(
        DriftItem::ColumnExtra { table: "Order".into(), column: "note".into() }.to_string(),
        "EXTRA    column Order.note"
    );
    assert_eq!(
        DriftItem::ColumnTypeDrift {
            table: "Order".into(), column: "amount".into(),
            snapshot: "Int".into(), live: "Decimal".into()
        }.to_string(),
        "TYPE     Order.amount  snapshot=Int  live=Decimal"
    );
    assert_eq!(
        DriftItem::NullabilityDrift {
            table: "Order".into(), column: "note".into(),
            snapshot: false, live: true
        }.to_string(),
        "NULLABLE Order.note  snapshot=false  live=true"
    );
}
