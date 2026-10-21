use std::{fs, path::Path, process::Command};

// BACKLOG items 377-382 - the database DSL and migration generator.
fn certo(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).current_dir(dir).args(args).output().unwrap();
    (out.status.success(),
     String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
     String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"))
}

fn project_with_migration(body: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("certo.toml"), "[project]\nname = \"m\"\n").unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    fs::write(dir.path().join("migrations").join("001_things.cto"),
        format!("module Migration\n\nmigration \"things\" {{\n    up {{\n{body}\n    }}\n    down {{\n        dropTable things\n    }}\n}}\n")).unwrap();
    dir
}

#[test]
fn migration_sql_has_terminators_real_defaults_and_real_types() {
    let dir = project_with_migration(r####"        rawSql "SELECT 1"
        createTable things {
            id: UUID primaryKey
            a: Int32
            b: Timestamp
            c: Int default -5
            d: Decimal default d"1.50"
            e: DateTime default now()
            f: UUID default gen_random_uuid()
            g: Float default -0.5
            h: Int8
            i: UInt
            j: Float32
            k: Bytes
            l: Int?
        }"####);
    let (ok, out, err) = certo(dir.path(), &["db", "migrate", "--dry-run"]);
    assert!(ok, "{err}");
    assert!(out.starts_with("SELECT 1;\n"), "{out}");
    for want in ["a INTEGER NOT NULL", "b TIMESTAMPTZ NOT NULL", "c BIGINT NOT NULL DEFAULT -5",
        "d NUMERIC NOT NULL DEFAULT 1.50", "e TIMESTAMPTZ NOT NULL DEFAULT now()",
        "f UUID NOT NULL DEFAULT gen_random_uuid()", "g DOUBLE PRECISION NOT NULL DEFAULT -0.5",
        "h SMALLINT NOT NULL", "i NUMERIC(20, 0) NOT NULL", "j REAL NOT NULL", "k BYTEA NOT NULL"] {
        assert!(out.contains(want), "missing `{want}` in:\n{out}");
    }
    assert!(out.contains("    l BIGINT\n") || out.contains("    l BIGINT,"), "an `Int?` column is nullable:\n{out}");
    assert!(!out.contains("DEFAULT NULL") && !out.contains("TEXT"), "{out}");
}

#[test]
fn a_default_that_cannot_be_expressed_is_an_error() {
    let dir = project_with_migration("        createTable things { id: UUID primaryKey, c: Int default (1 + 2) }");
    let (ok, _out, err) = certo(dir.path(), &["db", "migrate", "--dry-run"]);
    assert!(!ok);
    assert!(err.contains("the default for column `c`"), "{err}");
}

#[test]
fn db_create_numbers_migrations_in_creation_order() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("certo.toml"), "[project]\nname = \"m\"\n").unwrap();
    assert!(certo(dir.path(), &["db", "create", "create_products"]).0);
    assert!(certo(dir.path(), &["db", "create", "add_category_to_products"]).0);
    let mut names: Vec<String> = fs::read_dir(dir.path().join("migrations")).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    assert_eq!(names, vec!["001_create_products.cto", "002_add_category_to_products.cto"]);
}

#[test]
fn query_sql_is_snake_case_and_order_by_accumulates_keys() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r####"module P
import Stdlib.Db
import Stdlib.DbQuery
type OrderItems = { id: Int, customerId: Int, status: Text, totalAmount: Int }
type Customers = { id: Int, fullName: Text }
type Orders = { id: Int, customerId: Int, totalAmount: Int }
fn main(): Unit [io] = {
  val q = Query.from("OrderItems") |> Query.filter("customerId", "=", "1") |> Query.orderBy("status", "asc") |> Query.orderBy("totalAmount", "desc") |> Query.limit(5)
  println(Query.sql(q))
  val j = Query.from("Orders") |> Query.join("Customers", "Orders.customerId", "Customers.id") |> Query.groupBy("Customers.fullName") |> Query.aggregate("sum", "Orders.totalAmount", "total")
  println(Query.sql(j))
}
"####).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let out = String::from_utf8_lossy(&Command::new(&exe).output().unwrap().stdout).replace("\r\n", "\n");
    assert_eq!(out,
        "SELECT * FROM order_items AS order_items WHERE customer_id = $1 ORDER BY status ASC, total_amount DESC LIMIT 5\n\
         SELECT customers.full_name, SUM(orders.total_amount) AS total FROM orders AS orders JOIN customers AS customers ON orders.customer_id = customers.id GROUP BY customers.full_name\n");
}

#[test]
fn db_table_insert_sugar_calls_the_generated_insert_function() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Schema.cto"), "module Schema\nimport Stdlib.Db\n\ntype AuditLog = { id: Int, message: Text }\n\npub fn auditLogInsert(conn: Int, record: AuditLog): Int [io] =\n    dbExec(conn, \"INSERT INTO audit_log (id, message) VALUES ($1, $2)\", [intToText(record.id), record.message])\n").unwrap();
    fs::write(dir.path().join("main.cto"), "module Main\nimport Stdlib.Db\nimport Schema\n\nfn logIt(): Int [io] = db.auditLog.insert(AuditLog { id: 1, message: \"hi\" })\n\nfn main(): Unit [io] = { println(\"ok\") }\n").unwrap();
    let (ok, _o, err) = certo(dir.path(), &["check", "main.cto"]);
    assert!(ok, "{err}");
    let main = fs::read_to_string(dir.path().join("main.cto")).unwrap().replace(".insert(", ".save(");
    fs::write(dir.path().join("main.cto"), main).unwrap();
    let (ok, _o, err) = certo(dir.path(), &["check", "main.cto"]);
    assert!(!ok && err.contains("E0711") && err.contains("auditLogInsert"), "{err}");
}
