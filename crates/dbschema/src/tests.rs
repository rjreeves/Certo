use certo_ast::decl::{
    Decl, TypeDecl, TypeBody, RecordTypeDef, RecordFieldDef,
    MigrationDecl, MigrationOp, ColumnDef, AlterOp, FkAction,
};
use certo_ast::module::Module;
use certo_ast::span::{S, Span};
use certo_ast::types::{TypeExpr, ModulePath};
use crate::{check_module, DbErrorKind};

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
        is_priv_ctor: false,
        annotations: vec![],
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
            methods: vec![],
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

// BACKLOG item 313 — a migration written in the spec's own documented
// lowercase/snake_case table-name convention must still resolve against a
// `type` declared PascalCase (as `certo db pull` generates it).
#[test]
fn create_table_accepts_snake_case_name_for_pascal_case_type() {
    let m = module(vec![
        record_type("Categories", &[("id", "UUID"), ("name", "Text")]),
        migration("001_snake",
            vec![MigrationOp::CreateTable {
                name:    "categories".into(),
                columns: vec![col("id", "UUID"), col("name", "Text")],
                span:    DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "categories".into(), span: DUMMY }],
        ),
    ]);
    check_module(&m).unwrap();
}

// The same case-folding must apply to the column-type lookup, not just
// table existence — a wrong column type against a snake_case-named table
// must still be caught.
#[test]
fn column_type_mismatch_still_caught_through_snake_case_table_name() {
    let m = module(vec![
        record_type("Categories", &[("id", "UUID"), ("sortOrder", "Int")]),
        migration("001_snake",
            vec![MigrationOp::CreateTable {
                name:    "categories".into(),
                columns: vec![col("id", "UUID"), col("sortOrder", "Text")], // wrong type
                span:    DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "categories".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::ColumnTypeMismatch { column, .. } if column == "sortOrder")),
        "expected E0501: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
}

// A foreign-key target named in the migration's own snake_case convention
// must resolve against the PascalCase type it really refers to.
#[test]
fn foreign_key_target_accepts_snake_case_reference() {
    let m = module(vec![
        record_type("Categories", &[("id", "UUID")]),
        record_type("Products",   &[("id", "UUID"), ("categoryId", "UUID")]),
        migration("001_snake",
            vec![
                MigrationOp::CreateTable {
                    name: "categories".into(), columns: vec![col("id", "UUID")], span: DUMMY,
                },
                MigrationOp::CreateTable {
                    name: "products".into(),
                    columns: vec![col("id", "UUID"), col("categoryId", "UUID")],
                    span: DUMMY,
                },
                MigrationOp::AlterTable {
                    name: "products".into(),
                    ops:  vec![AlterOp::AddForeignKey {
                        column:     "categoryId".into(),
                        references: "categories".into(),
                        on_delete:  FkAction::SetNull,
                        span:       DUMMY,
                    }],
                    span: DUMMY,
                },
            ],
            vec![MigrationOp::DropTable { name: "products".into(), span: DUMMY },
                 MigrationOp::DropTable { name: "categories".into(), span: DUMMY }],
        ),
    ]);
    check_module(&m).unwrap();
}

// A table genuinely absent stays absent regardless of casing — the fold
// must not make every lookup vacuously succeed.
#[test]
fn snake_case_lookup_still_rejects_a_genuinely_unknown_table() {
    let m = module(vec![
        migration("001",
            vec![MigrationOp::CreateTable {
                name: "ghost_table".into(), columns: vec![], span: DUMMY,
            }],
            vec![MigrationOp::DropTable { name: "ghost_table".into(), span: DUMMY }],
        ),
    ]);
    let errs = check_module(&m).unwrap_err();
    assert!(has(&errs, |k| matches!(k, DbErrorKind::TableNotDeclaredAsType { table, .. } if table == "ghost_table")),
        "expected E0505: {:?}", errs.iter().map(|e| e.message()).collect::<Vec<_>>());
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

// ------------------------------------------------------------------ //
// decimal_bare_vs_param — BACKLOG item 128
// ------------------------------------------------------------------ //

#[test]
fn bare_decimal_compatible_with_parameterized_either_order() {
    assert!(crate::schema::decimal_bare_vs_param("Decimal", "Decimal(19, 4)"));
    assert!(crate::schema::decimal_bare_vs_param("Decimal(19, 4)", "Decimal"));
}

#[test]
fn different_decimal_params_not_compatible() {
    assert!(!crate::schema::decimal_bare_vs_param("Decimal(10, 2)", "Decimal(19, 4)"));
}

#[test]
fn same_decimal_params_not_flagged_by_this_helper() {
    // Exact matches are handled by the caller's own `==` check first — this
    // helper only ever returns true for a bare/parameterized *mismatch* in
    // form, so identical params correctly fall outside its job.
    assert!(!crate::schema::decimal_bare_vs_param("Decimal(19, 4)", "Decimal(19, 4)"));
}

#[test]
fn unrelated_types_not_compatible() {
    assert!(!crate::schema::decimal_bare_vs_param("Int", "Decimal(19, 4)"));
    assert!(!crate::schema::decimal_bare_vs_param("Decimal", "Text"));
}

#[test]
fn decimal_bare_vs_param_strips_nullable_suffix() {
    assert!(crate::schema::decimal_bare_vs_param("Decimal?", "Decimal(19, 4)?"));
}

// ------------------------------------------------------------------ //
// bounded_text_bare_vs_param — BACKLOG item 147
// ------------------------------------------------------------------ //

#[test]
fn bare_text_compatible_with_bounded_text_either_order() {
    assert!(crate::schema::bounded_text_bare_vs_param("Text", "BoundedText(255)"));
    assert!(crate::schema::bounded_text_bare_vs_param("BoundedText(255)", "Text"));
}

#[test]
fn different_bounded_text_lengths_not_compatible() {
    assert!(!crate::schema::bounded_text_bare_vs_param("BoundedText(10)", "BoundedText(255)"));
}

#[test]
fn same_bounded_text_lengths_not_flagged_by_this_helper() {
    assert!(!crate::schema::bounded_text_bare_vs_param("BoundedText(255)", "BoundedText(255)"));
}

#[test]
fn unrelated_types_not_compatible_bounded_text() {
    assert!(!crate::schema::bounded_text_bare_vs_param("Int", "BoundedText(255)"));
    assert!(!crate::schema::bounded_text_bare_vs_param("Text", "Decimal(19, 4)"));
}

#[test]
fn bounded_text_bare_vs_param_strips_nullable_suffix() {
    assert!(crate::schema::bounded_text_bare_vs_param("Text?", "BoundedText(255)?"));
}

// BACKLOG item 285 — `camel_to_snake` is the inverse of `snake_to_camel`/
// `snake_to_pascal` above, needed so migration DDL can round-trip with
// `certo db pull`'s own naming convention.
#[test]
fn camel_to_snake_converts_a_simple_field_name() {
    assert_eq!(crate::schema::camel_to_snake("customerId"), "customer_id");
}

#[test]
fn camel_to_snake_converts_a_pascal_case_table_name() {
    assert_eq!(crate::schema::camel_to_snake("ProductCategories"), "product_categories");
}

#[test]
fn camel_to_snake_is_a_no_op_on_an_already_lowercase_name() {
    assert_eq!(crate::schema::camel_to_snake("orders"), "orders");
}

#[test]
fn camel_to_snake_round_trips_with_snake_to_camel() {
    for name in ["customerId", "parentId", "sortOrder", "id", "categoryId"] {
        let snake = crate::schema::camel_to_snake(name);
        assert_eq!(crate::schema::snake_to_camel(&snake), name,
            "camel_to_snake({name:?}) = {snake:?} did not round-trip back via snake_to_camel");
    }
}
