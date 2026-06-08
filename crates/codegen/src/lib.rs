mod ty_to_c;
mod emit_mir;
mod emit_module;

pub use emit_module::{emit_module, CodegenOptions, RUNTIME_HEADER};

#[cfg(test)]
mod tests;
