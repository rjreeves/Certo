//! Live Postgres schema introspection via `information_schema`.
//!
//! Only compiled when the `live` feature is enabled (opt-in to avoid
//! pulling in the `postgres` crate for normal compilation).

use crate::schema::{Schema, SchemaColumn, SchemaTable};

/// Pull the schema for all user tables from a live Postgres database.
///
/// `conn_str` is a libpq connection string, e.g.
///   `"host=localhost dbname=oe user=postgres password=postgres1"`
pub fn pull_schema(conn_str: &str) -> Result<Schema, String> {
    let mut client = postgres::Client::connect(conn_str, postgres::NoTls)
        .map_err(|e| format!("connection failed: {e}"))?;

    // Pull all columns from user-defined tables (excluding system schemas).
    let rows = client.query(
        r#"
        SELECT
            c.table_name,
            c.column_name,
            c.data_type,
            c.udt_name,
            c.is_nullable
        FROM information_schema.columns c
        JOIN information_schema.tables t
          ON t.table_name   = c.table_name
         AND t.table_schema = c.table_schema
        WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema')
          AND t.table_type = 'BASE TABLE'
        ORDER BY c.table_name, c.ordinal_position
        "#,
        &[],
    ).map_err(|e| format!("query failed: {e}"))?;

    let mut schema = Schema { version: 1, ..Default::default() };

    for row in &rows {
        let table_name: &str  = row.get(0);
        let col_name:   &str  = row.get(1);
        let data_type:  &str  = row.get(2);
        let udt_name:   &str  = row.get(3);
        let is_nullable: &str = row.get(4);

        let certo_ty = pg_type_to_certo(data_type, udt_name);
        let nullable  = is_nullable == "YES";

        // Pascal-case the table name to match Certo type naming.
        let type_name = snake_to_pascal(table_name);

        let table = schema.tables
            .entry(type_name.clone())
            .or_insert_with(|| SchemaTable { name: type_name, columns: Vec::new() });

        table.columns.push(SchemaColumn {
            name: col_name.to_string(),
            ty:   certo_ty,
            nullable,
        });
    }

    Ok(schema)
}

/// Map Postgres `data_type` / `udt_name` to a Certo type string.
fn pg_type_to_certo(data_type: &str, udt_name: &str) -> String {
    match data_type {
        "integer" | "smallint"                  => "Int".into(),
        "bigint"                                => "Int".into(),
        "numeric" | "decimal" | "real"
            | "double precision"                => "Decimal".into(),
        "boolean"                               => "Bool".into(),
        "text" | "character varying" | "varchar"
            | "character" | "char" | "name"     => "Text".into(),
        "uuid"                                  => "UUID".into(),
        "timestamp with time zone"
            | "timestamptz"                     => "Timestamp".into(),
        "timestamp without time zone"
            | "timestamp"                       => "Timestamp".into(),
        "date"                                  => "Date".into(),
        "json" | "jsonb"                        => "Json".into(),
        "bytea"                                 => "Bytes".into(),
        // USER-DEFINED = enum or custom domain — use the udt_name as the type.
        "USER-DEFINED"                          => snake_to_pascal(udt_name),
        // Arrays
        "ARRAY"                                 => format!("List<{}>", snake_to_pascal(udt_name.trim_start_matches('_'))),
        _                                       => format!("/* {} */", data_type),
    }
}

fn snake_to_pascal(s: &str) -> String {
    s.split('_').map(|w| {
        let mut c = w.chars();
        c.next().map(|ch| ch.to_uppercase().to_string() + c.as_str()).unwrap_or_default()
    }).collect()
}
