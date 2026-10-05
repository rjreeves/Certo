//! The generated Rust (`tests/generated`) compiled and run: against SQLite always, against PostgreSQL when
//! `CERTO_TEST_PG_URL` is set (WARNING: that database's `public` schema is dropped and recreated).
//! The fixtures are made from `rust_codegen_cases.rs`; they are rewritten with
//! `cargo test -p certo-ql --test rust_codegen_bless -- --ignored`.

#[path = "rust_codegen_cases.rs"]
mod cases;
#[path = "generated/pg.rs"]
#[allow(dead_code)]
mod gen_pg;
#[path = "generated/sqlite.rs"]
#[allow(dead_code)]
mod gen_sqlite;
#[path = "generated/mysql.rs"]
#[allow(dead_code)]
mod gen_mysql;
#[path = "generated/pg_async.rs"]
#[allow(dead_code)]
mod gen_pg_async;
#[path = "generated/mysql_async.rs"]
#[allow(dead_code)]
mod gen_mysql_async;

use certo_sql::Dialect;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use std::str::FromStr;

fn ddl(dialect: Dialect) -> String {
    let (ir, d) = certo_sdl::compile(cases::SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let empty = certo_sdl::SchemaIR::empty();
    let plan = certo_mdl::diff(&empty, &schema);
    match dialect {
        Dialect::Mysql => unreachable!("MySQL runs its DDL statement by statement (see generated_mysql_code_runs)"),
        Dialect::Postgres => certo_sql::render(&plan, dialect).unwrap(),
        Dialect::Sqlite => certo_sql::render_with(&plan, dialect, certo_sql::Schemas { old: &empty, new: &schema }).unwrap(),
    }
}

#[test]
fn the_fixtures_are_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generated");
    for (file, dialect, async_) in [
        ("pg.rs", Dialect::Postgres, false),
        ("sqlite.rs", Dialect::Sqlite, false),
        ("mysql.rs", Dialect::Mysql, false),
        ("pg_async.rs", Dialect::Postgres, true),
        ("mysql_async.rs", Dialect::Mysql, true),
    ] {
        let (ir, _) = certo_sdl::compile(cases::SCHEMA);
        let schema = ir.unwrap();
        let queries = if dialect == Dialect::Mysql { cases::QUERIES_MYSQL } else { cases::QUERIES };
        let (s, d) = certo_ql::compile(&schema, queries, dialect);
        let fresh = certo_ql::generate_rust(&schema, &s.unwrap_or_else(|| panic!("{d:?}")), &certo_ql::RustOptions { dialect, async_ });
        let on_disk = std::fs::read_to_string(dir.join(file)).unwrap();
        assert_eq!(on_disk.replace("\r\n", "\n"), fresh, "{file} is out of date: rewrite it with the rust_codegen_bless test");
    }
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}
fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}
fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
}

#[test]
fn generated_sqlite_code_runs() {
    use gen_sqlite::*;
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.execute_batch(&ddl(Dialect::Sqlite)).unwrap();
    let token = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let meta = Json(r#"{"plan":"pro"}"#.into());

    let a = add_customer(&conn, "Ann", Some("a@x.com"), Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(&meta), Some(token)).unwrap();
    let b = add_customer(&conn, "Bob", None, Role::User, None, None, None, None).unwrap();
    assert_eq!((a[0].id, b[0].id), (1, 2));

    let all = customers_by_role(&conn, None).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        all[0],
        CustomersByRoleRow {
            id: 1,
            name: "Ann".into(),
            email: Some("a@x.com".into()),
            role: Role::Admin,
            balance: Some(dec("100.50")),
            born: Some(day(1990, 1, 1)),
            meta: Some(meta.clone()),
            token: Some(token),
        }
    );
    assert_eq!((all[1].email.clone(), all[1].balance, all[1].born, all[1].meta.clone(), all[1].token), (None, None, None, None, None));
    assert_eq!(customers_by_role(&conn, Some(Role::User)).unwrap().iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Bob"]);

    assert_eq!(rename_customer(&conn, 2, "Robert").unwrap(), 1);
    assert_eq!(rename_customer(&conn, 99, "Nobody").unwrap(), 0);
    assert_eq!(customers_by_role(&conn, Some(Role::User)).unwrap()[0].name, "Robert");

    let early = at(2026, 1, 2, 3);
    let seen = day(2026, 1, 2).and_hms_opt(4, 5, 6).unwrap();
    let o1 = place_order(&conn, 1, dec("10.00"), 2, true, Some(early), Some(seen)).unwrap();
    let o2 = place_order(&conn, 1, dec("25.50"), 1, false, None, None).unwrap();
    assert_eq!((o1[0].id, o2[0].id), (1, 2));
    let orders = orders_since(&conn, at(2000, 1, 1, 0), None).unwrap();
    assert_eq!(orders.len(), 2);
    assert_eq!((orders[0].total, orders[0].qty, orders[0].paid), (dec("10.00"), 2, true));
    assert_eq!((orders[0].shipped, orders[0].seen), (Some(early), Some(seen)));
    assert_eq!((orders[1].total, orders[1].shipped, orders[1].seen), (dec("25.50"), None, None));
    assert_eq!(orders[1].created, o2[0].created);
    assert!(orders_since(&conn, at(2999, 1, 1, 0), None).unwrap().is_empty(), "the bound timestamp compares with stored ones");

    let blob = add_blob(&conn, Some(&[0, 1, 2, 255]), 2.5, 0.25).unwrap();
    add_blob(&conn, None, -1.0, 0.0).unwrap();
    let rows = blobs(&conn).unwrap();
    assert_eq!(blob[0].id, 1);
    assert_eq!((rows[0].data.clone(), rows[0].ratio, rows[0].score), (Some(vec![0, 1, 2, 255]), Some(2.5), Some(0.25)));
    assert_eq!(rows[1].data, None);

    assert_eq!(purge_orders(&conn).unwrap(), 2);
    assert!(orders_since(&conn, at(2000, 1, 1, 0), None).unwrap().is_empty());
}

#[test]
fn generated_postgres_code_runs() {
    use gen_pg::*;
    let Ok(url) = std::env::var("CERTO_TEST_PG_URL") else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let mut client = postgres::Client::connect(&url, postgres::NoTls).expect("connect");
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").unwrap();
    client.batch_execute(&ddl(Dialect::Postgres)).unwrap();
    let token = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let meta = Json(r#"{"plan": "pro"}"#.into());

    let a = add_customer(&mut client, "Ann", Some("a@x.com"), Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(&meta), Some(token)).unwrap();
    let b = add_customer(&mut client, "Bob", None, Role::User, None, None, None, None).unwrap();
    assert_eq!((a[0].id, b[0].id), (1, 2));

    let all = customers_by_role(&mut client, None).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!((all[0].role, all[0].balance, all[0].born, all[0].token), (Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(token)));
    assert_eq!(all[0].meta.as_ref().map(|j| j.0.replace(' ', "")), Some(r#"{"plan":"pro"}"#.to_string()));
    assert_eq!((all[1].email.clone(), all[1].balance, all[1].meta.clone(), all[1].token), (None, None, None, None));
    assert_eq!(customers_by_role(&mut client, Some(Role::User)).unwrap().iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Bob"]);

    assert_eq!(rename_customer(&mut client, 2, "Robert").unwrap(), 1);
    assert_eq!(rename_customer(&mut client, 99, "Nobody").unwrap(), 0);

    let early = at(2026, 1, 2, 3);
    let seen = day(2026, 1, 2).and_hms_opt(4, 5, 6).unwrap();
    place_order(&mut client, 1, dec("10.00"), 2, true, Some(early), Some(seen)).unwrap();
    let o2 = place_order(&mut client, 1, dec("25.50"), 1, false, None, None).unwrap();
    let orders = orders_since(&mut client, at(2000, 1, 1, 0), None).unwrap();
    assert_eq!(orders.len(), 2);
    assert_eq!((orders[0].total, orders[0].qty, orders[0].paid, orders[0].shipped, orders[0].seen), (dec("10.00"), 2, true, Some(early), Some(seen)));
    assert_eq!((orders[1].shipped, orders[1].seen, orders[1].created), (None, None, o2[0].created));
    assert!(orders_since(&mut client, at(2999, 1, 1, 0), None).unwrap().is_empty());

    // inside a transaction, which is a GenericClient too
    let mut tx = client.transaction().unwrap();
    add_blob(&mut tx, Some(&[0, 1, 2, 255]), 2.5, 0.25).unwrap();
    add_blob(&mut tx, None, -1.0, 0.0).unwrap();
    let rows = blobs(&mut tx).unwrap();
    assert_eq!((rows[0].data.clone(), rows[0].ratio, rows[0].score), (Some(vec![0, 1, 2, 255]), Some(2.5), Some(0.25)));
    assert_eq!(rows[1].data, None);
    tx.rollback().unwrap();
    assert!(blobs(&mut client).unwrap().is_empty());

    assert_eq!(purge_orders(&mut client).unwrap(), 2);
}

#[test]
fn generated_mysql_code_runs() {
    use gen_mysql::*;
    use mysql::prelude::Queryable;
    let Ok(url) = std::env::var("CERTO_TEST_MYSQL_URL") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let opts = mysql::Opts::from_url(&url).expect("url");
    let mut admin = mysql::Conn::new(mysql::OptsBuilder::from_opts(opts.clone()).db_name(Some("mysql".to_string()))).expect("connect");
    admin.query_drop("DROP DATABASE IF EXISTS certo_rust_codegen").unwrap();
    admin.query_drop("CREATE DATABASE certo_rust_codegen").unwrap();
    let mut conn = mysql::Conn::new(mysql::OptsBuilder::from_opts(opts).db_name(Some("certo_rust_codegen".to_string()))).expect("connect");
    let (ir, d) = certo_sdl::compile(cases::SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let empty = certo_sdl::SchemaIR::empty();
    let plan = certo_mdl::diff(&empty, &schema);
    for s in certo_sql::lower_with(&plan, Dialect::Mysql, certo_sql::Schemas { old: &empty, new: &schema }).unwrap() {
        conn.query_drop(&s).unwrap_or_else(|e| panic!("{e}
{s}"));
    }
    let token = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let meta = Json(r#"{"plan":"pro"}"#.into());

    assert_eq!(add_customer(&mut conn, "Ann", Some("a@x.com"), Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(&meta), Some(token)).unwrap(), 1);
    assert_eq!(add_customer(&mut conn, "Bob", None, Role::User, None, None, None, None).unwrap(), 1);

    let all = customers_by_role(&mut conn, None).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!((all[0].id, all[0].name.as_str(), all[0].email.as_deref(), all[0].role), (1, "Ann", Some("a@x.com"), Role::Admin));
    assert_eq!((all[0].balance, all[0].born, all[0].token), (Some(dec("100.50")), Some(day(1990, 1, 1)), Some(token)));
    assert_eq!(all[0].meta.as_ref().map(|j| j.0.replace(' ', "")), Some(r#"{"plan":"pro"}"#.to_string()));
    assert_eq!((all[1].email.clone(), all[1].balance, all[1].born, all[1].meta.clone(), all[1].token), (None, None, None, None, None));
    assert_eq!(customers_by_role(&mut conn, Some(Role::User)).unwrap().iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Bob"]);

    assert_eq!(rename_customer(&mut conn, 2, "Robert").unwrap(), 1);
    assert_eq!(rename_customer(&mut conn, 99, "Nobody").unwrap(), 0);

    let early = at(2026, 1, 2, 3);
    let seen = day(2026, 1, 2).and_hms_opt(4, 5, 6).unwrap();
    assert_eq!(place_order(&mut conn, 1, dec("10.00"), 2, true, Some(early), Some(seen)).unwrap(), 1);
    assert_eq!(place_order(&mut conn, 1, dec("25.50"), 1, false, None, None).unwrap(), 1);
    let orders = orders_since(&mut conn, at(2000, 1, 1, 0), None).unwrap();
    assert_eq!(orders.len(), 2);
    assert_eq!((orders[0].total, orders[0].qty, orders[0].paid, orders[0].shipped, orders[0].seen), (dec("10.00"), 2, true, Some(early), Some(seen)));
    assert_eq!((orders[1].total, orders[1].shipped, orders[1].seen), (dec("25.50"), None, None));
    assert!(orders[1].created > at(2000, 1, 1, 0), "now() is a UTC timestamp");
    assert!(orders_since(&mut conn, at(2999, 1, 1, 0), None).unwrap().is_empty());

    // inside a transaction (anything Queryable)
    let mut tx = conn.start_transaction(mysql::TxOpts::default()).unwrap();
    assert_eq!(add_blob(&mut tx, Some(&[0, 1, 2, 255]), 2.5, 0.25).unwrap(), 1);
    assert_eq!(add_blob(&mut tx, None, -1.0, 0.0).unwrap(), 1);
    let rows = blobs(&mut tx).unwrap();
    assert_eq!((rows[0].data.clone(), rows[0].ratio, rows[0].score), (Some(vec![0, 1, 2, 255]), Some(2.5), Some(0.25)));
    assert_eq!(rows[1].data, None);
    tx.rollback().unwrap();
    assert!(blobs(&mut conn).unwrap().is_empty());

    assert_eq!(purge_orders(&mut conn).unwrap(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn generated_async_postgres_code_runs() {
    use gen_pg_async::*;
    let Ok(url) = std::env::var("CERTO_TEST_PG_URL") else {
        eprintln!("CERTO_TEST_PG_URL not set; skipping live PostgreSQL test");
        return;
    };
    let (mut client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.expect("connect");
    tokio::spawn(connection);
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    client.batch_execute(&ddl(Dialect::Postgres)).await.unwrap();
    let token = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let meta = Json(r#"{"plan": "pro"}"#.into());

    let a = add_customer(&client, "Ann", Some("a@x.com"), Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(&meta), Some(token)).await.unwrap();
    let b = add_customer(&client, "Bob", None, Role::User, None, None, None, None).await.unwrap();
    assert_eq!((a[0].id, b[0].id), (1, 2));
    let all = customers_by_role(&client, None).await.unwrap();
    assert_eq!((all[0].role, all[0].balance, all[0].born, all[0].token), (Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(token)));
    assert_eq!(all[0].meta.as_ref().map(|j| j.0.replace(' ', "")), Some(r#"{"plan":"pro"}"#.to_string()));
    assert_eq!((all[1].email.clone(), all[1].balance, all[1].meta.clone(), all[1].token), (None, None, None, None));
    assert_eq!(customers_by_role(&client, Some(Role::User)).await.unwrap().len(), 1);
    assert_eq!(rename_customer(&client, 2, "Robert").await.unwrap(), 1);
    assert_eq!(rename_customer(&client, 99, "Nobody").await.unwrap(), 0);

    let early = at(2026, 1, 2, 3);
    let seen = day(2026, 1, 2).and_hms_opt(4, 5, 6).unwrap();
    place_order(&client, 1, dec("10.00"), 2, true, Some(early), Some(seen)).await.unwrap();
    let o2 = place_order(&client, 1, dec("25.50"), 1, false, None, None).await.unwrap();
    let orders = orders_since(&client, at(2000, 1, 1, 0), None).await.unwrap();
    assert_eq!((orders[0].total, orders[0].qty, orders[0].paid, orders[0].shipped, orders[0].seen), (dec("10.00"), 2, true, Some(early), Some(seen)));
    assert_eq!((orders[1].shipped, orders[1].seen, orders[1].created), (None, None, o2[0].created));

    // inside a transaction, which is a GenericClient too
    let tx = client.transaction().await.unwrap();
    add_blob(&tx, Some(&[0, 1, 2, 255]), 2.5, 0.25).await.unwrap();
    assert_eq!(blobs(&tx).await.unwrap().len(), 1);
    tx.rollback().await.unwrap();
    assert!(blobs(&client).await.unwrap().is_empty());
    assert_eq!(purge_orders(&client).await.unwrap(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn generated_async_mysql_code_runs() {
    use gen_mysql_async::*;
    use mysql_async::prelude::Queryable;
    let Ok(url) = std::env::var("CERTO_TEST_MYSQL_URL") else {
        eprintln!("CERTO_TEST_MYSQL_URL not set; skipping live MySQL test");
        return;
    };
    let opts = mysql_async::Opts::from_url(&url).expect("url");
    let mut admin = mysql_async::Conn::new(mysql_async::OptsBuilder::from_opts(opts.clone()).db_name(Some("mysql"))).await.expect("connect");
    admin.query_drop("DROP DATABASE IF EXISTS certo_rust_async").await.unwrap();
    admin.query_drop("CREATE DATABASE certo_rust_async").await.unwrap();
    let mut conn = mysql_async::Conn::new(mysql_async::OptsBuilder::from_opts(opts).db_name(Some("certo_rust_async"))).await.expect("connect");
    let (ir, d) = certo_sdl::compile(cases::SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let empty = certo_sdl::SchemaIR::empty();
    let plan = certo_mdl::diff(&empty, &schema);
    for s in certo_sql::lower_with(&plan, Dialect::Mysql, certo_sql::Schemas { old: &empty, new: &schema }).unwrap() {
        conn.query_drop(&s).await.unwrap_or_else(|e| panic!("{e}\n{s}"));
    }
    let token = uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let meta = Json(r#"{"plan":"pro"}"#.into());

    assert_eq!(add_customer(&mut conn, "Ann", Some("a@x.com"), Role::Admin, Some(dec("100.50")), Some(day(1990, 1, 1)), Some(&meta), Some(token)).await.unwrap(), 1);
    assert_eq!(add_customer(&mut conn, "Bob", None, Role::User, None, None, None, None).await.unwrap(), 1);
    let all = customers_by_role(&mut conn, None).await.unwrap();
    assert_eq!((all[0].id, all[0].name.as_str(), all[0].role), (1, "Ann", Role::Admin));
    assert_eq!((all[0].balance, all[0].born, all[0].token), (Some(dec("100.50")), Some(day(1990, 1, 1)), Some(token)));
    assert_eq!(all[0].meta.as_ref().map(|j| j.0.replace(' ', "")), Some(r#"{"plan":"pro"}"#.to_string()));
    assert_eq!((all[1].email.clone(), all[1].balance, all[1].born, all[1].meta.clone(), all[1].token), (None, None, None, None, None));
    assert_eq!(rename_customer(&mut conn, 2, "Robert").await.unwrap(), 1);
    assert_eq!(rename_customer(&mut conn, 99, "Nobody").await.unwrap(), 0);

    let early = at(2026, 1, 2, 3);
    let seen = day(2026, 1, 2).and_hms_opt(4, 5, 6).unwrap();
    assert_eq!(place_order(&mut conn, 1, dec("10.00"), 2, true, Some(early), Some(seen)).await.unwrap(), 1);
    assert_eq!(place_order(&mut conn, 1, dec("25.50"), 1, false, None, None).await.unwrap(), 1);
    let orders = orders_since(&mut conn, at(2000, 1, 1, 0), None).await.unwrap();
    assert_eq!((orders[0].total, orders[0].qty, orders[0].paid, orders[0].shipped, orders[0].seen), (dec("10.00"), 2, true, Some(early), Some(seen)));
    assert_eq!((orders[1].shipped, orders[1].seen), (None, None));

    // inside a transaction (anything Queryable)
    let mut tx = conn.start_transaction(mysql_async::TxOpts::default()).await.unwrap();
    assert_eq!(add_blob(&mut tx, Some(&[0, 1, 2, 255]), 2.5, 0.25).await.unwrap(), 1);
    assert_eq!(blobs(&mut tx).await.unwrap().len(), 1);
    tx.rollback().await.unwrap();
    assert!(blobs(&mut conn).await.unwrap().is_empty());
    assert_eq!(purge_orders(&mut conn).await.unwrap(), 2);
}
