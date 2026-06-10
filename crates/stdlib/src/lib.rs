mod core;
mod collections;
mod text;
mod datetime;
mod money;
mod db;
mod env;
mod file;
mod path;
mod process;
mod json;
mod http;
mod math;
pub mod seed;

pub use seed::seed_stdlib;

pub use core::{CORE_C, CORE_CERTO};
pub use collections::{COLLECTIONS_C, COLLECTIONS_CERTO};
pub use text::{TEXT_C, TEXT_CERTO};
pub use datetime::{DATETIME_C, DATETIME_CERTO};
pub use money::{MONEY_C, MONEY_CERTO};
pub use db::{DB_C, DB_CERTO};
pub use env::{ENV_C, ENV_CERTO};
pub use file::{FILE_C, FILE_CERTO};
pub use path::{PATH_C, PATH_CERTO};
pub use process::{PROCESS_C, PROCESS_CERTO};
pub use json::{JSON_C, JSON_CERTO};
pub use http::{HTTP_C, HTTP_CERTO};
pub use math::{MATH_C, MATH_CERTO};

/// The full C runtime header: base types + all stdlib implementations.
pub fn full_c_runtime() -> String {
    [CORE_C, COLLECTIONS_C, TEXT_C, DATETIME_C, MONEY_C,
     ENV_C, FILE_C, PATH_C, PROCESS_C, JSON_C, HTTP_C, MATH_C].concat()
}

/// Full C runtime including optional PostgreSQL support.
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
        ("Stdlib.Env",         ENV_CERTO),
        ("Stdlib.File",        FILE_CERTO),
        ("Stdlib.Path",        PATH_CERTO),
        ("Stdlib.Process",     PROCESS_CERTO),
        ("Stdlib.Json",        JSON_CERTO),
        ("Stdlib.Http",        HTTP_CERTO),
        ("Stdlib.Math",        MATH_CERTO),
    ]
}

#[cfg(test)]
mod tests;
