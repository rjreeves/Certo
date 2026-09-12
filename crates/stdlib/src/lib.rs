mod core;
mod bytes;
mod credential;
mod collections;
mod channel;
mod result;
mod text;
mod datetime;
mod money;
mod db;
mod dbquery;
mod dbmutation;
mod env;
mod file;
mod path;
mod process;
mod host;
mod cli;
mod json;
mod http;
mod math;
mod crypto;
mod regex;
mod csv;
mod uuid;
pub mod seed;
mod effects_seed;

pub use seed::seed_stdlib;
pub use effects_seed::seed_stdlib_effects;

pub use core::CORE_C;
pub use bytes::BYTES_C;
pub use credential::CREDENTIAL_C;
pub use collections::COLLECTIONS_C;
pub use channel::CHANNEL_C;
pub use result::RESULT_C;
pub use text::TEXT_C;
pub use datetime::DATETIME_C;
pub use money::MONEY_C;
pub use db::DB_C;
pub use dbquery::DBQUERY_C;
pub use dbmutation::DBMUTATION_C;
pub use env::ENV_C;
pub use file::FILE_C;
pub use path::PATH_C;
pub use process::PROCESS_C;
pub use host::HOST_C;
pub use cli::CLI_C;
pub use json::JSON_C;
pub use http::HTTP_C;
pub use math::MATH_C;
pub use crypto::CRYPTO_C;
pub use regex::REGEX_C;
pub use csv::CSV_C;
pub use uuid::UUID_C;

/// The full C runtime header: base types + all stdlib implementations.
pub fn full_c_runtime() -> String {
    [CORE_C, BYTES_C, CREDENTIAL_C, COLLECTIONS_C, CHANNEL_C, RESULT_C, TEXT_C, DATETIME_C, MONEY_C,
     ENV_C, FILE_C, PATH_C, PROCESS_C, HOST_C, CLI_C, JSON_C, HTTP_C, MATH_C,
     CRYPTO_C, REGEX_C, CSV_C, UUID_C].concat()
}

/// Full C runtime including optional PostgreSQL support.
pub fn full_c_runtime_with_db(with_db: bool) -> String {
    let base = full_c_runtime();
    if with_db { base + DB_C + DBQUERY_C + DBMUTATION_C } else { base }
}

#[cfg(test)]
mod tests;
