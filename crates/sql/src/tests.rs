use crate::*;
use certo_mdl::diff;
use certo_sdl::{compile, SchemaIR};

fn ir(src: &str) -> SchemaIR {
    let (ir, d) = compile(src);
    ir.unwrap_or_else(|| panic!("schema failed: {d:?}"))
}

fn sql(before: &str, after: &str) -> Vec<String> {
    let b = if before.is_empty() { SchemaIR::empty() } else { ir(before) };
    lower(&diff(&b, &ir(after)), Dialect::Postgres).unwrap()
}

fn sql_err(before: &str, after: &str) -> LowerError {
    lower(&diff(&ir(before), &ir(after)), Dialect::Postgres).unwrap_err()
}

#[test]
fn initial_migration() {
    let out = sql(
        "",
        "enum Role { admin, user }
         type Address { street: text }
         table users {
             id: uuid primary key default gen_uuid()
             email: text not null unique
             role: Role default user
             home: Address
             created: timestamp default now()
         }
         table posts {
             id: uuid primary key
             author_id: uuid not null references users on delete cascade
         }
         index users_email on users (email)
         constraint adult on users using role != admin",
    );
    let expected = [
        "CREATE TYPE \"Role\" AS ENUM ('admin', 'user');",
        "CREATE TYPE \"Address\" AS (\"street\" text);",
        "CREATE TABLE \"posts\" (\n    \"id\" uuid NOT NULL,\n    \"author_id\" uuid NOT NULL,\n    CONSTRAINT \"posts_pkey\" PRIMARY KEY (\"id\")\n);",
        "CREATE TABLE \"users\" (\n    \"id\" uuid NOT NULL DEFAULT gen_random_uuid(),\n    \"email\" text NOT NULL,\n    \"role\" \"Role\" DEFAULT 'user',\n    \"home\" \"Address\",\n    \"created\" timestamptz DEFAULT now(),\n    CONSTRAINT \"users_pkey\" PRIMARY KEY (\"id\"),\n    CONSTRAINT \"users_email_key\" UNIQUE (\"email\")\n);",
        "ALTER TABLE \"posts\" ADD CONSTRAINT \"fk_posts_author_id\" FOREIGN KEY (\"author_id\") REFERENCES \"users\" (\"id\") ON DELETE CASCADE;",
        "CREATE INDEX \"users_email\" ON \"users\" (\"email\");",
        "ALTER TABLE \"users\" ADD CONSTRAINT \"adult\" CHECK (\"role\" <> 'admin');",
    ];
    assert_eq!(out, expected);
}

#[test]
fn composite_primary_key() {
    let out = sql("", "table t { a: int primary key b: int primary key }");
    assert!(out[0].contains("PRIMARY KEY (\"a\", \"b\")"), "{}", out[0]);
}

#[test]
fn add_and_drop_columns() {
    let base = "table t { id: int primary key } ";
    let out = sql(base, "table t { id: int primary key  n: int not null default 0  u: text unique }");
    assert_eq!(out, [
        "ALTER TABLE \"t\" ADD COLUMN \"n\" integer NOT NULL DEFAULT 0;",
        "ALTER TABLE \"t\" ADD COLUMN \"u\" text UNIQUE;",
    ]);
    let out = sql("table t { id: int primary key n: int }", "table t { id: int primary key }");
    assert_eq!(out, ["ALTER TABLE \"t\" DROP COLUMN \"n\";"]);
}

#[test]
fn alter_column_variants() {
    // nullability
    assert_eq!(
        sql("table t { id: int primary key x: int }", "table t { id: int primary key x: int not null }"),
        ["ALTER TABLE \"t\" ALTER COLUMN \"x\" SET NOT NULL;"]
    );
    // tightening with a default backfills existing NULLs first
    assert_eq!(
        sql("table t { id: int primary key x: int }", "table t { id: int primary key x: int not null default 7 }"),
        [
            "UPDATE \"t\" SET \"x\" = 7 WHERE \"x\" IS NULL;",
            "ALTER TABLE \"t\" ALTER COLUMN \"x\" SET NOT NULL;",
            "ALTER TABLE \"t\" ALTER COLUMN \"x\" SET DEFAULT 7;",
        ]
    );
    assert_eq!(
        sql("table t { id: int primary key x: int not null }", "table t { id: int primary key x: int }"),
        ["ALTER TABLE \"t\" ALTER COLUMN \"x\" DROP NOT NULL;"]
    );
    // default set / dropped
    assert_eq!(
        sql("table t { id: int primary key x: int }", "table t { id: int primary key x: int default 5 }"),
        ["ALTER TABLE \"t\" ALTER COLUMN \"x\" SET DEFAULT 5;"]
    );
    assert_eq!(
        sql("table t { id: int primary key x: int default 5 }", "table t { id: int primary key x: int }"),
        ["ALTER TABLE \"t\" ALTER COLUMN \"x\" DROP DEFAULT;"]
    );
    // unique
    assert_eq!(
        sql("table t { id: int primary key x: text }", "table t { id: int primary key x: text unique }"),
        ["ALTER TABLE \"t\" ADD CONSTRAINT \"t_x_key\" UNIQUE (\"x\");"]
    );
    // type change with a default: drop it, retype, restore it
    assert_eq!(
        sql("table t { id: int primary key x: int default 1 }", "table t { id: int primary key x: bigint default 1 }"),
        [
            "ALTER TABLE \"t\" ALTER COLUMN \"x\" DROP DEFAULT;",
            "ALTER TABLE \"t\" ALTER COLUMN \"x\" TYPE bigint USING \"x\"::bigint;",
            "ALTER TABLE \"t\" ALTER COLUMN \"x\" SET DEFAULT 1;",
        ]
    );
    // to an enum goes through text
    let out = sql(
        "enum E { a } table t { id: int primary key x: text }",
        "enum E { a } table t { id: int primary key x: E }",
    );
    assert_eq!(out, ["ALTER TABLE \"t\" ALTER COLUMN \"x\" TYPE \"E\" USING \"x\"::text::\"E\";"]);
}

#[test]
fn foreign_key_lifecycle() {
    let a = "table u { id: int primary key } table t { id: int primary key r: int }";
    let b = "table u { id: int primary key } table t { id: int primary key r: int references u on delete set null on update cascade }";
    assert_eq!(sql(a, b), [
        "ALTER TABLE \"t\" ADD CONSTRAINT \"fk_t_r\" FOREIGN KEY (\"r\") REFERENCES \"u\" (\"id\") ON DELETE SET NULL ON UPDATE CASCADE;",
    ]);
    assert_eq!(sql(b, a), ["ALTER TABLE \"t\" DROP CONSTRAINT \"fk_t_r\";"]);
}

#[test]
fn enums_and_types() {
    assert_eq!(sql("enum E { a }", "enum E { a, b }"), ["ALTER TYPE \"E\" ADD VALUE IF NOT EXISTS 'b';"]);
    assert_eq!(sql("enum E { a }", ""), ["DROP TYPE \"E\";"]);
    assert_eq!(
        sql("type T { a: int b: int }", "type T { a: bigint c: text }"),
        [
            "ALTER TYPE \"T\" DROP ATTRIBUTE \"b\";",
            "ALTER TYPE \"T\" ALTER ATTRIBUTE \"a\" TYPE bigint;",
            "ALTER TYPE \"T\" ADD ATTRIBUTE \"c\" text;",
        ]
    );
}

#[test]
fn unsupported_ops_name_themselves() {
    let e = sql_err("enum E { a, b }", "enum E { a }");
    assert_eq!(e.op, "- enum variant E.b");
    assert!(e.reason.contains("cannot remove an enum value"));
    let e = sql_err("table t { id: int primary key }", "table t { id: int }");
    assert!(e.reason.contains("primary key"), "{e}");
}

#[test]
fn quoting_and_expressions() {
    // reserved words and quotes are safe
    let out = sql("", "table order { id: int primary key  user: text default \"it's\" }");
    assert!(out[0].contains("CREATE TABLE \"order\""));
    assert!(out[0].contains("\"user\" text DEFAULT 'it''s'"));
    // arithmetic / boolean constraints
    let out = sql(
        "table t { id: int primary key a: int b: int }",
        "table t { id: int primary key a: int b: int } constraint c on t using a + 1 * 2 > b and a != 0 or b == 3",
    );
    assert_eq!(out, [
        "ALTER TABLE \"t\" ADD CONSTRAINT \"c\" CHECK ((((\"a\" + (1 * 2)) > \"b\") AND (\"a\" <> 0)) OR (\"b\" = 3));",
    ]);
}

#[test]
fn relationships_produce_no_sql() {
    let out = sql("", "table u { id: int primary key } table t { id: int primary key r: u -> one }");
    assert_eq!(out.len(), 2); // just the two CREATE TABLEs
}

#[test]
fn dialect_names() {
    assert_eq!(Dialect::from_name("pg"), Some(Dialect::Postgres));
    assert_eq!(Dialect::from_name("oracle"), None);
}

#[test]
fn enum_additions_run_outside_the_transaction() {
    let plan = diff(
        &ir("enum E { a } table t { id: int primary key e: E }"),
        &ir("enum E { a, b } table t { id: int primary key e: E default b  x: int }"),
    );
    let batches = lower_batches(&plan, Dialect::Postgres).unwrap();
    assert_eq!(batches.len(), 2);
    assert!(!batches[0].transactional);
    assert_eq!(batches[0].statements, ["ALTER TYPE \"E\" ADD VALUE IF NOT EXISTS 'b';"]);
    assert!(batches[1].transactional);
    assert_eq!(batches[1].statements.len(), 2); // add column x, set default
    let script = render(&plan, Dialect::Postgres).unwrap();
    assert!(script.starts_with("ALTER TYPE \"E\" ADD VALUE IF NOT EXISTS 'b';
BEGIN;"), "{script}");
    assert!(script.ends_with("COMMIT;"));
    // nothing to do -> nothing rendered
    assert_eq!(render(&diff(&SchemaIR::empty(), &SchemaIR::empty()), Dialect::Postgres).unwrap(), "");
}

// ---- MDL-driven ops --------------------------------------------------- //

fn mig(before: &str, after: &str, mdl: &str) -> Vec<String> {
    let (plan, d) = certo_mdl::compile_migration(&ir(before), &ir(after), mdl);
    let plan = plan.unwrap_or_else(|| panic!("{d:?}"));
    lower(&plan, Dialect::Postgres).unwrap()
}

#[test]
fn rename_table_renames_derived_constraints() {
    let out = mig(
        "table u { id: uuid primary key } table a { id: uuid primary key  k: text unique  u_id: uuid references u }",
        "table u { id: uuid primary key } table b { id: uuid primary key  k: text unique  u_id: uuid references u }",
        "rename table a -> b",
    );
    assert_eq!(out, [
        "ALTER TABLE \"a\" RENAME TO \"b\";",
        "ALTER TABLE \"b\" RENAME CONSTRAINT \"a_pkey\" TO \"b_pkey\";",
        "ALTER TABLE \"b\" RENAME CONSTRAINT \"a_k_key\" TO \"b_k_key\";",
        "ALTER TABLE \"b\" RENAME CONSTRAINT \"fk_a_u_id\" TO \"fk_b_u_id\";",
    ]);
}

#[test]
fn rename_column_renames_derived_constraints() {
    let out = mig(
        "table u { id: uuid primary key } table t { id: uuid primary key  k: text unique  r: uuid references u }",
        "table u { id: uuid primary key } table t { id: uuid primary key  key2: text unique  r2: uuid references u }",
        "rename column t.k -> key2\nrename column t.r -> r2",
    );
    assert_eq!(out, [
        "ALTER TABLE \"t\" RENAME COLUMN \"k\" TO \"key2\";",
        "ALTER TABLE \"t\" RENAME CONSTRAINT \"t_k_key\" TO \"t_key2_key\";",
        "ALTER TABLE \"t\" RENAME COLUMN \"r\" TO \"r2\";",
        "ALTER TABLE \"t\" RENAME CONSTRAINT \"fk_t_r\" TO \"fk_t_r2\";",
    ]);
}

#[test]
fn recreate_enum_remaps_rows_and_swaps_types() {
    let out = mig(
        "enum Role { admin, user, guest } table users { id: int primary key  role: Role default guest }",
        "enum Role { admin, user } table users { id: int primary key  role: Role default user }",
        "remap Role.guest -> user",
    );
    assert_eq!(out, [
        "ALTER TABLE \"users\" ALTER COLUMN \"role\" SET DEFAULT 'user';",
        // recreate: drop default, remap rows, build the smaller type, move, restore, swap
        "ALTER TABLE \"users\" ALTER COLUMN \"role\" DROP DEFAULT;",
        "UPDATE \"users\" SET \"role\" = 'user' WHERE \"role\" = 'guest';",
        "CREATE TYPE \"Role__new\" AS ENUM ('admin', 'user');",
        "ALTER TABLE \"users\" ALTER COLUMN \"role\" TYPE \"Role__new\" USING \"role\"::text::\"Role__new\";",
        "ALTER TABLE \"users\" ALTER COLUMN \"role\" SET DEFAULT 'user';",
        "DROP TYPE \"Role\";",
        "ALTER TYPE \"Role__new\" RENAME TO \"Role\";",
    ]);
}

#[test]
fn backfill_and_data_steps() {
    let out = mig(
        "table t { id: int primary key  name: text }",
        "table t { id: int primary key  name: text  slug: text not null }",
        "backfill t.slug = lower(name)
         before { update t set name = \"n/a\" where is_null(name) }
         after { update t set name = upper(name), slug = \"x\" where id > 1 }",
    );
    assert_eq!(out, [
        "UPDATE \"t\" SET \"name\" = 'n/a' WHERE (\"name\" IS NULL);",
        "ALTER TABLE \"t\" ADD COLUMN \"slug\" text;",
        "UPDATE \"t\" SET \"slug\" = lower(\"name\") WHERE \"slug\" IS NULL;",
        "ALTER TABLE \"t\" ALTER COLUMN \"slug\" SET NOT NULL;",
        "UPDATE \"t\" SET \"name\" = upper(\"name\"), \"slug\" = 'x' WHERE (\"id\" > 1);",
    ]);
}

#[test]
fn raw_sql_is_dialect_filtered() {
    let s = "table t { id: int primary key }";
    let out = mig(s, s, "after { sql postgres \"ANALYZE t\"  sql \"SELECT 1;\" }");
    assert_eq!(out, ["ANALYZE t;", "SELECT 1;"]);
    // a step for another dialect emits nothing here
    let plan = certo_mdl::MigrationPlan {
        version: certo_mdl::PLAN_VERSION,
        ops: vec![certo_mdl::Op::RawSql { dialect: Some("mysql".into()), sql: "X".into() }],
    };
    assert!(lower(&plan, Dialect::Postgres).unwrap().is_empty());
}

#[test]
fn removing_an_enum_variant_points_at_remap() {
    let e = sql_err("enum E { a, b } table t { id: int primary key }", "enum E { a } table t { id: int primary key }");
    assert!(e.reason.contains("remap"), "{e}");
}


// ---- widened SDL ------------------------------------------------------------ //

#[test]
fn new_types_lower_to_their_postgres_names() {
    let out = sql(
        "",
        "table t {
            id: int primary key
            a: smallint  b: real  c: timestamp_naive  d: varchar(255)  e: char(3)
            f: decimal(10,2)  g: decimal(8)  h: decimal
        }",
    );
    let t = &out[0];
    for expected in [
        "\"a\" smallint",
        "\"b\" real",
        "\"c\" timestamp without time zone",
        "\"d\" character varying(255)",
        "\"e\" character(3)",
        "\"f\" numeric(10,2)",
        "\"g\" numeric(8,0)",
        "\"h\" numeric,",
    ] {
        assert!(t.contains(expected), "expected `{expected}` in:\n{t}");
    }
}

#[test]
fn new_literals_and_functions_lower() {
    let out = sql(
        "",
        "table t {
            id: int primary key
            n: int default -5
            f: float default -1.5
            d: decimal(10,2) default 19.99
            born: date default today()
        }",
    );
    let t = &out[0];
    // negatives are parenthesised so they are safe next to any operator
    assert!(t.contains("\"n\" integer DEFAULT (-5)"), "{t}");
    assert!(t.contains("\"f\" double precision DEFAULT (-1.5)"), "{t}");
    assert!(t.contains("\"d\" numeric(10,2) DEFAULT 19.99"), "{t}");
    assert!(t.contains("\"born\" date DEFAULT CURRENT_DATE"), "{t}");
}

#[test]
fn new_constraint_forms_lower() {
    let out = sql(
        "table t { id: int primary key  n: int  s: text  r: text }",
        "table t { id: int primary key  n: int  s: text  r: text }
         constraint c1 on t using not n == 1 and s is not null
         constraint c2 on t using n in (1, 2, -3) and r not in (\"a\", \"b\")
         constraint c3 on t using n is null or trim(s) != \"\" and round(n) > 0 and nullif(n, 0) > 1",
    );
    assert_eq!(out, [
        "ALTER TABLE \"t\" ADD CONSTRAINT \"c1\" CHECK ((NOT (\"n\" = 1)) AND (\"s\" IS NOT NULL));",
        "ALTER TABLE \"t\" ADD CONSTRAINT \"c2\" CHECK ((\"n\" IN (1, 2, (-3))) AND (\"r\" NOT IN ('a', 'b')));",
        "ALTER TABLE \"t\" ADD CONSTRAINT \"c3\" CHECK ((\"n\" IS NULL) OR (((btrim(\"s\") <> '') AND (round(\"n\") > 0)) AND (nullif(\"n\", 0) > 1)));",
    ]);
}

#[test]
fn changing_a_parameterised_type_is_an_alter_type() {
    let out = sql(
        "table t { id: int primary key  s: varchar(20) }",
        "table t { id: int primary key  s: varchar(50) }",
    );
    assert_eq!(out, ["ALTER TABLE \"t\" ALTER COLUMN \"s\" TYPE character varying(50) USING \"s\"::character varying(50);"]);
}


// ---- serial, identity, sequences -------------------------------------------- //

#[test]
fn serial_and_identity_columns_are_created() {
    let out = sql("", "table t { a: serial primary key  b: bigserial unique  c: smallserial  d: bigint generated always  e: int generated by default }");
    let t = &out[0];
    assert!(t.contains("\"a\" serial NOT NULL"), "{t}");
    assert!(t.contains("\"b\" bigserial NOT NULL"), "{t}");
    assert!(t.contains("\"c\" smallserial NOT NULL"), "{t}");
    assert!(t.contains("\"d\" bigint GENERATED ALWAYS AS IDENTITY NOT NULL"), "{t}");
    assert!(t.contains("\"e\" integer GENERATED BY DEFAULT AS IDENTITY NOT NULL"), "{t}");
    assert!(!t.contains("DEFAULT nextval"), "serial has no explicit default");
}

#[test]
fn sequences_and_nextval_defaults() {
    let out = sql(
        "",
        "sequence order_seq start 1000 increment 5 cycle cache 10
         sequence down_seq increment -2 min -50 max -1 start -3
         table t { id: bigint primary key default nextval(order_seq) }",
    );
    assert_eq!(out[0], "CREATE SEQUENCE \"down_seq\" AS bigint START WITH -3 INCREMENT BY -2 MINVALUE -50 MAXVALUE -1 CACHE 1 NO CYCLE;");
    assert_eq!(out[1], "CREATE SEQUENCE \"order_seq\" AS bigint START WITH 1000 INCREMENT BY 5 MINVALUE 1 MAXVALUE 9223372036854775807 CACHE 10 CYCLE;");
    assert!(out[2].starts_with("CREATE TABLE \"t\""), "the tables come after the sequences they use");
    assert!(out[2].contains("\"id\" bigint NOT NULL DEFAULT nextval('\"order_seq\"')"), "{}", out[2]);

    assert_eq!(
        sql("sequence s", "sequence s increment 5 cycle"),
        ["ALTER SEQUENCE \"s\" START WITH 1 INCREMENT BY 5 MINVALUE 1 MAXVALUE 9223372036854775807 CACHE 1 CYCLE;"]
    );
    assert_eq!(sql("sequence s", ""), ["DROP SEQUENCE \"s\";"]);
}

#[test]
fn adding_a_serial_column_or_generation() {
    assert_eq!(
        sql("table t { id: int primary key }", "table t { id: int primary key  n: serial }"),
        ["ALTER TABLE \"t\" ADD COLUMN \"n\" serial NOT NULL;"]
    );
    // an existing NOT NULL column becomes serial: build the owned sequence and catch it up
    let out = sql("table t { id: int primary key  n: int not null }", "table t { id: int primary key  n: serial }");
    assert_eq!(out, [
        "CREATE SEQUENCE \"t_n_seq\" AS integer OWNED BY \"t\".\"n\";",
        "ALTER TABLE \"t\" ALTER COLUMN \"n\" SET DEFAULT nextval('\"t_n_seq\"');",
        "SELECT setval('\"t_n_seq\"', COALESCE((SELECT max(\"n\") FROM \"t\"), 0) + 1, false);",
    ]);
    let out = sql("table t { id: int primary key  n: bigint not null }", "table t { id: int primary key  n: bigserial }");
    assert!(out[0].contains("AS bigint OWNED BY"), "{}", out[0]);
    // ...or an identity
    let out = sql("table t { id: int primary key  n: int not null }", "table t { id: int primary key  n: int generated always }");
    assert_eq!(out, [
        "ALTER TABLE \"t\" ALTER COLUMN \"n\" ADD GENERATED ALWAYS AS IDENTITY;",
        "SELECT setval(pg_get_serial_sequence('\"t\"', 'n'), COALESCE((SELECT max(\"n\") FROM \"t\"), 0) + 1, false);",
    ]);
}

#[test]
fn removing_and_switching_generations() {
    let ident = "table t { id: int primary key  n: int generated always }";
    let by_default = "table t { id: int primary key  n: int generated by default }";
    let serial = "table t { id: int primary key  n: serial }";
    let none = "table t { id: int primary key  n: int not null }";
    assert_eq!(sql(ident, by_default), ["ALTER TABLE \"t\" ALTER COLUMN \"n\" SET GENERATED BY DEFAULT;"]);
    assert_eq!(sql(by_default, ident), ["ALTER TABLE \"t\" ALTER COLUMN \"n\" SET GENERATED ALWAYS;"]);
    assert_eq!(sql(ident, none), ["ALTER TABLE \"t\" ALTER COLUMN \"n\" DROP IDENTITY;"]);
    // the serial sequence is found through its ownership, so renames cannot break it
    let out = sql(serial, none);
    assert_eq!(out.len(), 1);
    assert!(out[0].starts_with("DO $$ DECLARE s text := pg_get_serial_sequence('\"t\"', 'n');"), "{}", out[0]);
    assert!(out[0].contains("DROP DEFAULT") && out[0].contains("EXECUTE 'DROP SEQUENCE ' || s"), "{}", out[0]);
    // serial <-> identity: drop one, build the other
    let out = sql(serial, ident);
    assert!(out[0].starts_with("DO $$") && out[1].contains("ADD GENERATED ALWAYS AS IDENTITY"), "{out:?}");
    let out = sql(ident, serial);
    assert_eq!(out[0], "ALTER TABLE \"t\" ALTER COLUMN \"n\" DROP IDENTITY;");
    assert!(out[1].starts_with("CREATE SEQUENCE \"t_n_seq\""), "{out:?}");
}

#[test]
fn views_lower_to_create_and_drop_view() {
    let t = "table users { id: uuid primary key  email: text not null  age: int }";
    let with = format!("{t} view adults on users (id, email) where age >= 18 view all_mail on users (email)");
    let out = sql("", &with);
    assert_eq!(
        &out[out.len() - 2..],
        [
            "CREATE VIEW \"adults\" AS SELECT \"id\", \"email\" FROM \"users\" WHERE (\"age\" >= 18);",
            "CREATE VIEW \"all_mail\" AS SELECT \"email\" FROM \"users\";",
        ]
    );
    assert_eq!(sql(&with, t), ["DROP VIEW \"adults\";", "DROP VIEW \"all_mail\";"]);
}

// ---- MySQL ---------------------------------------------------------------- //

fn mysql_sql(before: &str, after: &str) -> Vec<String> {
    let b = if before.is_empty() { SchemaIR::empty() } else { ir(before) };
    let a = ir(after);
    lower_with(&diff(&b, &a), Dialect::Mysql, Schemas { old: &b, new: &a }).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn mysql_tables_use_the_binary_collation_and_tag_ambiguous_types() {
    let out = mysql_sql(
        "",
        "enum Role { admin, user_ }
         table users {
             id: serial primary key
             token: uuid default gen_uuid()
             email: varchar(100) not null unique
             role: Role not null default user_
             at: timestamp
             on_: timestamp_naive
             price: decimal
         }",
    );
    assert_eq!(out.len(), 1);
    let t = &out[0];
    for expected in [
        "CREATE TABLE `users` (",
        "`id` INT NOT NULL AUTO_INCREMENT",
        "`token` CHAR(36) NULL DEFAULT (LOWER(CONCAT(HEX(RANDOM_BYTES(4))",
        "COMMENT 'certo:uuid'",
        "`email` VARCHAR(100) NOT NULL",
        "`role` ENUM('admin','user_') NOT NULL DEFAULT ('user_') COMMENT 'certo:enum Role'",
        "`at` DATETIME(6) NULL COMMENT 'certo:timestamptz'",
        "`on_` DATETIME(6) NULL,",
        "`price` DECIMAL(65,30) NULL",
        "PRIMARY KEY (`id`)",
        "CONSTRAINT `users_email_key` UNIQUE (`email`)",
        ") ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;",
    ] {
        assert!(t.contains(expected), "expected `{expected}` in:\n{t}");
    }
}

#[test]
fn mysql_alters_restate_the_whole_column() {
    let before = "table t { id: int primary key  n: int  s: varchar(10) }";
    let out = mysql_sql(before, "table t { id: int primary key  n: bigint not null default 0  s: varchar(10) unique }");
    assert_eq!(
        out,
        [
            "UPDATE `t` SET `n` = 0 WHERE `n` IS NULL;",
            "ALTER TABLE `t` MODIFY COLUMN `n` BIGINT NOT NULL DEFAULT (0);",
            "ALTER TABLE `t` ADD CONSTRAINT `t_s_key` UNIQUE (`s`);",
        ]
    );
    assert_eq!(mysql_sql(before, "table t { id: int primary key  n: int }"), ["ALTER TABLE `t` DROP COLUMN `s`;"]);
    // a new enum value redefines the columns that use it
    let e1 = "enum E { a } table t { id: int primary key  e: E }";
    let out = mysql_sql(e1, "enum E { a, b } table t { id: int primary key  e: E }");
    assert_eq!(out, ["ALTER TABLE `t` MODIFY COLUMN `e` ENUM('a','b') NULL COMMENT 'certo:enum E';"]);
}

#[test]
fn mysql_strings_escape_backslashes() {
    use certo_sdl::ExprIR;
    assert_eq!(render_expr(Dialect::Mysql, &ExprIR::String { value: r"a\b'c".into() }), r"'a\\b''c'");
    assert_eq!(quote_ident(Dialect::Mysql, "we`ird"), "`we``ird`");
}
