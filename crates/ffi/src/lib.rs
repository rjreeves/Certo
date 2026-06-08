//! `certo-ffi` — C header generation and REST client codegen for Certo.
//!
//! # Modes
//!
//! ## C header  (`--header`)
//!
//! Takes a `.certo` source file and emits a `.h` with C prototypes for every
//! `pub fn` declaration.  Useful when embedding Certo-compiled code in a C/C++
//! project or exposing a shared library.
//!
//! ```sh
//! certo-ffi --header src/math.certo -o include/math.h
//! ```
//!
//! ## REST client  (`--rest-client`)
//!
//! Takes a JSON schema file describing a REST API and emits a `.certo` source
//! file with typed `pub fn` stubs and `[async, io]` effects.
//!
//! ```sh
//! certo-ffi --rest-client api/users.json -o src/UserApi.certo
//! ```

pub mod error;
pub mod ty;
pub mod header;
pub mod rest;

pub use error::FfiError;
pub use header::generate_header;
pub use rest::{parse_schema, generate_client, RestSchema};
