mod core;
mod collections;
mod text;
mod datetime;
mod money;
mod db;
pub mod seed;

pub use seed::seed_stdlib;

pub use core::{CORE_C, CORE_CERTO};
pub use collections::{COLLECTIONS_C, COLLECTIONS_CERTO};
pub use text::{TEXT_C, TEXT_CERTO};
pub use datetime::{DATETIME_C, DATETIME_CERTO};
pub use money::{MONEY_C, MONEY_CERTO};
pub use db::{DB_C, DB_CERTO};

/// The full C runtime header: base types + all stdlib implementations.
///
/// Concatenate this with `certo_codegen::RUNTIME_HEADER` (or use in its
/// place) when compiling generated code that uses stdlib functions.
pub fn full_c_runtime() -> String {
    [CORE_C, COLLECTIONS_C, TEXT_C, DATETIME_C, MONEY_C].concat()
}

/// Full C runtime including optional PostgreSQL support.
/// Pass `with_db = true` when the program imports Stdlib.Db.
pub fn full_c_runtime_with_db(with_db: bool) -> String {
    let base = full_c_runtime();
    if with_db { base + DB_C } else { base }
}

/// All stdlib Certo source files, keyed by module path.
pub fn certo_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Stdlib.Core",        CORE_CERTO),
        ("Stdlib.Collections", COLLECTIONS_CERTO),
        ("Stdlib.Text",        TEXT_CERTO),
        ("Stdlib.DateTime",    DATETIME_CERTO),
        ("Stdlib.Money",       MONEY_CERTO),
        ("Stdlib.Db",          DB_CERTO),
    ]
}

#[cfg(test)]
mod tests;
