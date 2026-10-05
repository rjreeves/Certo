//! Writes the generated Rust fixtures: `CERTO_BLESS=1 cargo test -p certo-ql --test rust_codegen_bless -- --ignored`.
//! (They are separate from the tests that use them so the fixtures can be rewritten while they do not compile.)

#[path = "rust_codegen_cases.rs"]
mod cases;

use certo_sql::Dialect;

pub fn generate(dialect: Dialect) -> String {
    let (ir, d) = certo_sdl::compile(cases::SCHEMA);
    let schema = ir.unwrap_or_else(|| panic!("{d:?}"));
    let (s, d) = certo_ql::compile(&schema, cases::QUERIES, dialect);
    let s = s.unwrap_or_else(|| panic!("{d:?}"));
    certo_ql::generate_rust(&schema, &s, &certo_ql::RustOptions { dialect })
}

#[test]
#[ignore]
fn write_the_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generated");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pg.rs"), generate(Dialect::Postgres)).unwrap();
    std::fs::write(dir.join("sqlite.rs"), generate(Dialect::Sqlite)).unwrap();
}
