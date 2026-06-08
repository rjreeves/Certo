mod core;
mod collections;
mod text;
mod datetime;
mod money;
pub mod seed;

pub use seed::seed_stdlib;

pub use core::{CORE_C, CORE_CERTO};
pub use collections::{COLLECTIONS_C, COLLECTIONS_CERTO};
pub use text::{TEXT_C, TEXT_CERTO};
pub use datetime::{DATETIME_C, DATETIME_CERTO};
pub use money::{MONEY_C, MONEY_CERTO};

/// The full C runtime header: base types + all stdlib implementations.
///
/// Concatenate this with `certo_codegen::RUNTIME_HEADER` (or use in its
/// place) when compiling generated code that uses stdlib functions.
pub fn full_c_runtime() -> String {
    [CORE_C, COLLECTIONS_C, TEXT_C, DATETIME_C, MONEY_C].concat()
}

/// All stdlib Certo source files, keyed by module path.
pub fn certo_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Stdlib.Core",        CORE_CERTO),
        ("Stdlib.Collections", COLLECTIONS_CERTO),
        ("Stdlib.Text",        TEXT_CERTO),
        ("Stdlib.DateTime",    DATETIME_CERTO),
        ("Stdlib.Money",       MONEY_CERTO),
    ]
}

#[cfg(test)]
mod tests;
