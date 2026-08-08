use certo_ast::decl::{MigrationDecl, MigrationOp, ColumnDef, AlterOp, FkAction};
use certo_ast::types::{TypeExpr, ModulePath};
use certo_ast::span::{S, Span};
use crate::sql_gen::op_to_sql;
use crate::state::MigrationState;
use crate::plan::{plan_up, plan_down, Direction};
use crate::runner::{plan_sql, commit_steps};

fn dummy_span() -> Span { Span { start: 0, end: 0 } }
fn s<T>(v: T) -> S<T> { S { node: v, span: dummy_span() } }

fn int_ty() -> S<TypeExpr> {
    s(TypeExpr::Named {
        path: ModulePath { segments: vec![s("Int".to_string())], span: dummy_span() },
        args: vec![],
        span: dummy_span(),
    })
}

fn text_ty() -> S<TypeExpr> {
    s(TypeExpr::Named {
        path: ModulePath { segments: vec![s("Text".to_string())], span: dummy_span() },
        args: vec![],
        span: dummy_span(),
    })
}

fn bool_ty() -> S<TypeExpr> {
    s(TypeExpr::Named {
        path: ModulePath { segments: vec![s("Bool".to_string())], span: dummy_span() },
        args: vec![],
        span: dummy_span(),
    })
}

fn make_col(name: &str, ty: S<TypeExpr>, pk: bool, nullable: bool) -> ColumnDef {
    ColumnDef { name: name.to_string(), ty, primary_key: pk, nullable, unique: false, default: None, span: dummy_span() }
}

// ------------------------------------------------------------------ //
// sql_gen tests
// ------------------------------------------------------------------ //

#[test]
fn create_table_basic() {
    let op = MigrationOp::CreateTable {
        name: "users".to_string(),
        columns: vec![
            make_col("id",    int_ty(),  true,  false),
            make_col("email", text_ty(), false, false),
        ],
        span: dummy_span(),
    };
    let sql = op_to_sql(&op);
    assert!(sql.contains("CREATE TABLE users"), "got: {}", sql);
    assert!(sql.contains("id BIGINT NOT NULL PRIMARY KEY"), "got: {}", sql);
    assert!(sql.contains("email TEXT NOT NULL"), "got: {}", sql);
}

#[test]
fn drop_table_generates_sql() {
    let op = MigrationOp::DropTable { name: "old_table".to_string(), span: dummy_span() };
    assert_eq!(op_to_sql(&op), "DROP TABLE old_table;");
}

#[test]
fn create_index_generates_sql() {
    let op = MigrationOp::CreateIndex {
        name:    "idx_email".to_string(),
        table:   "users".to_string(),
        columns: vec!["email".to_string()],
        span:    dummy_span(),
    };
    let sql = op_to_sql(&op);
    assert_eq!(sql, "CREATE INDEX idx_email ON users (email);");
}

#[test]
fn drop_index_generates_sql() {
    let op = MigrationOp::DropIndex { name: "idx_email".to_string(), span: dummy_span() };
    assert_eq!(op_to_sql(&op), "DROP INDEX idx_email;");
}

#[test]
fn raw_sql_passthrough() {
    let op = MigrationOp::RawSql { sql: "VACUUM FULL;".to_string(), span: dummy_span() };
    assert_eq!(op_to_sql(&op), "VACUUM FULL;");
}

#[test]
fn alter_table_add_column() {
    let op = MigrationOp::AlterTable {
        name: "users".to_string(),
        ops:  vec![AlterOp::AddColumn { def: make_col("active", bool_ty(), false, false) }],
        span: dummy_span(),
    };
    let sql = op_to_sql(&op);
    assert!(sql.contains("ALTER TABLE users"), "got: {}", sql);
    assert!(sql.contains("ADD COLUMN active BOOLEAN"), "got: {}", sql);
}

#[test]
fn alter_table_drop_column() {
    let op = MigrationOp::AlterTable {
        name: "users".to_string(),
        ops:  vec![AlterOp::DropColumn { name: "old_col".to_string(), span: dummy_span() }],
        span: dummy_span(),
    };
    let sql = op_to_sql(&op);
    assert!(sql.contains("DROP COLUMN old_col"), "got: {}", sql);
}

#[test]
fn alter_table_add_foreign_key() {
    let op = MigrationOp::AlterTable {
        name: "orders".to_string(),
        ops:  vec![AlterOp::AddForeignKey {
            column:     "user_id".to_string(),
            references: "users".to_string(),
            on_delete:  FkAction::Cascade,
            span:       dummy_span(),
        }],
        span: dummy_span(),
    };
    let sql = op_to_sql(&op);
    assert!(sql.contains("ADD FOREIGN KEY (user_id) REFERENCES users ON DELETE CASCADE"), "got: {}", sql);
}

// ------------------------------------------------------------------ //
// State tests
// ------------------------------------------------------------------ //

#[test]
fn state_starts_empty() {
    let state = MigrationState::default();
    assert!(!state.is_applied("m001"));
}

#[test]
fn state_mark_applied() {
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    assert!(state.is_applied("m001"));
    assert!(!state.is_applied("m002"));
}

#[test]
fn state_rollback() {
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    state.mark_applied("m002");
    state.mark_rolled_back("m001");
    assert!(!state.is_applied("m001"));
    assert!(state.is_applied("m002"));
}

#[test]
fn state_round_trip_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("migrations.json");
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    state.save(&path).unwrap();
    let loaded = MigrationState::load(&path).unwrap();
    assert!(loaded.is_applied("m001"));
}

// ------------------------------------------------------------------ //
// Plan tests
// ------------------------------------------------------------------ //

fn make_migration(name: &str) -> MigrationDecl {
    MigrationDecl {
        name:        name.to_string(),
        description: None,
        up:          vec![],
        down:        vec![],
        span:        dummy_span(),
    }
}

#[test]
fn plan_up_all_pending() {
    let migrations = vec![make_migration("m001"), make_migration("m002")];
    let state = MigrationState::default();
    let steps = plan_up(&migrations, &state);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].migration.name, "m001");
    assert!(matches!(steps[0].direction, Direction::Up));
}

#[test]
fn plan_up_skips_applied() {
    let migrations = vec![make_migration("m001"), make_migration("m002")];
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    let steps = plan_up(&migrations, &state);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].migration.name, "m002");
}

#[test]
fn plan_down_one() {
    let migrations = vec![make_migration("m001"), make_migration("m002")];
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    state.mark_applied("m002");
    let steps = plan_down(&migrations, &state, 1);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].migration.name, "m002"); // most-recently-applied first
    assert!(matches!(steps[0].direction, Direction::Down));
}

#[test]
fn plan_up_nothing_to_do() {
    let migrations = vec![make_migration("m001")];
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    let steps = plan_up(&migrations, &state);
    assert!(steps.is_empty());
}

// ------------------------------------------------------------------ //
// plan_sql / commit_steps tests
//
// `plan_sql` must be pure (no state mutation) — `--dry-run` relies on that.
// `commit_steps` must only ever be called by the CLI after real execution
// succeeded; these tests just verify it persists exactly the given steps,
// not that it's wired to real execution (that's CLI-level, not this crate's
// concern — this crate has no process-spawning/psql code at all).
// ------------------------------------------------------------------ //

fn make_migration_with_ops(name: &str) -> MigrationDecl {
    MigrationDecl {
        name:        name.to_string(),
        description: None,
        up:          vec![MigrationOp::CreateTable {
            name: "widgets".to_string(),
            columns: vec![make_col("id", int_ty(), true, false)],
            span: dummy_span(),
        }],
        down:        vec![MigrationOp::DropTable { name: "widgets".to_string(), span: dummy_span() }],
        span:        dummy_span(),
    }
}

#[test]
fn plan_sql_generates_real_ddl_for_up_steps() {
    let migrations = vec![make_migration_with_ops("m001")];
    let state = MigrationState::default();
    let steps = plan_up(&migrations, &state);
    let sql = plan_sql(&steps);
    assert_eq!(sql.len(), 1);
    assert!(sql[0].contains("CREATE TABLE widgets"), "got: {}", sql[0]);
}

#[test]
fn plan_sql_generates_real_ddl_for_down_steps() {
    let migrations = vec![make_migration_with_ops("m001")];
    let mut state = MigrationState::default();
    state.mark_applied("m001");
    let steps = plan_down(&migrations, &state, 1);
    let sql = plan_sql(&steps);
    assert_eq!(sql.len(), 1);
    assert_eq!(sql[0], "DROP TABLE widgets;");
}

#[test]
fn plan_sql_does_not_mutate_state() {
    // Regression guard: plan_sql takes no MigrationState/manifest at all —
    // calling it must never have a side effect on migration state, unlike
    // the old run_steps(dry_run: false) which marked-applied unconditionally.
    let migrations = vec![make_migration_with_ops("m001")];
    let state = MigrationState::default();
    let steps = plan_up(&migrations, &state);
    let _ = plan_sql(&steps);
    assert!(!state.is_applied("m001"));
}

#[test]
fn commit_steps_persists_applied_state() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("migrations.json");
    let migrations = vec![make_migration_with_ops("m001")];
    let state = MigrationState::default();
    let steps = plan_up(&migrations, &state);

    commit_steps(&steps, &manifest).unwrap();

    let loaded = MigrationState::load(&manifest).unwrap();
    assert!(loaded.is_applied("m001"));
}

#[test]
fn commit_steps_persists_rolled_back_state() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("migrations.json");
    let mut initial = MigrationState::default();
    initial.mark_applied("m001");
    initial.save(&manifest).unwrap();

    let migrations = vec![make_migration_with_ops("m001")];
    let state = MigrationState::load(&manifest).unwrap();
    let steps = plan_down(&migrations, &state, 1);

    commit_steps(&steps, &manifest).unwrap();

    let loaded = MigrationState::load(&manifest).unwrap();
    assert!(!loaded.is_applied("m001"));
}

// ------------------------------------------------------------------ //
// End-to-end: source text → parsed structured ops → real SQL.
// Guards against regression of the old `format!("{expr:?}")` stub that
// emitted Rust AST debug output instead of SQL.
// ------------------------------------------------------------------ //

fn parse_migration_ops(src: &str) -> Vec<MigrationOp> {
    let module = certo_parser::parse(src).expect("parse error");
    for d in module.decls {
        if let certo_ast::decl::Decl::Migration(m) = d.node {
            return m.up;
        }
    }
    panic!("no migration found");
}

#[test]
fn end_to_end_create_table_emits_ddl() {
    let ops = parse_migration_ops(
        "module A\nmigration \"init\" {\n\
           up { createTable users { id: UUID primaryKey, email: Text unique } }\n\
           down { dropTable users }\n\
         }");
    let sql = op_to_sql(&ops[0]);
    assert!(sql.starts_with("CREATE TABLE users ("), "got: {sql}");
    assert!(sql.contains("id UUID"), "got: {sql}");
    assert!(sql.contains("PRIMARY KEY"), "got: {sql}");
    assert!(sql.contains("email TEXT"), "got: {sql}");
    assert!(sql.contains("UNIQUE"), "got: {sql}");
    // The old stub leaked Rust debug syntax — make sure that never returns.
    assert!(!sql.contains("Call {") && !sql.contains("Expr"), "leaked AST debug: {sql}");
}

#[test]
fn end_to_end_alter_and_index_emit_ddl() {
    let ops = parse_migration_ops(
        "module A\nmigration \"m\" {\n\
           up {\n\
             alterTable posts { addColumn views: Int foreignKey authorId references users onDelete cascade }\n\
             createIndex posts_author_idx on posts [authorId]\n\
           }\n\
           down { dropIndex posts_author_idx }\n\
         }");
    let alter = op_to_sql(&ops[0]);
    assert!(alter.contains("ALTER TABLE posts"), "got: {alter}");
    assert!(alter.contains("ADD COLUMN views BIGINT"), "got: {alter}");
    assert!(alter.to_uppercase().contains("FOREIGN KEY"), "got: {alter}");
    assert!(alter.to_uppercase().contains("ON DELETE CASCADE"), "got: {alter}");

    let index = op_to_sql(&ops[1]);
    assert_eq!(index, "CREATE INDEX posts_author_idx ON posts (authorId);");
}
