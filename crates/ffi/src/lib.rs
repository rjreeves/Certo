//! `certo-ffi` — C header generation and REST client codegen for Certo.
//!
//! # Modes
//!
//! ## C header  (`--header`)
//!
//! Takes a `.cto` source file and emits a `.h` with C prototypes for every
//! `pub fn` declaration.  Useful when embedding Certo-compiled code in a C/C++
//! project or exposing a shared library.
//!
//! ```sh
//! certo-ffi --header src/math.cto -o include/math.h
//! ```
//!
//! ## REST client  (`--rest-client`)
//!
//! Takes a JSON schema file describing a REST API — including named `models`
//! (typed records, with generated JSON encode/decode functions) — and emits
//! a `.cto` source file with typed `pub fn` stubs returning `Result<T, Text>`,
//! `[io]`-effectful.
//!
//! ```sh
//! certo-ffi --rest-client api/users.json -o src/UserApi.cto
//! ```
//!
//! ## OpenAPI client  (`--openapi`)
//!
//! Same output as `--rest-client`, generated from an OpenAPI 3.x JSON
//! document instead of the hand-written schema format — reads `paths` and
//! `components.schemas` and produces the identical `RestSchema` shape (see
//! `openapi` module docs for the exact subset supported).
//!
//! ```sh
//! certo-ffi --openapi openapi.json -o src/StripeApi.cto
//! ```

pub mod error;
pub mod ty;
pub mod header;
pub mod rest;
pub mod openapi;

pub use error::FfiError;
pub use header::generate_header;
pub use rest::{parse_schema, generate_client, RestSchema};
pub use openapi::parse_openapi;
