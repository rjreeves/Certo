mod ty_to_c;
mod emit_mir;
mod emit_module;

pub use emit_module::{emit_module, CodegenOptions, RUNTIME_HEADER};
pub use emit_mir::emit_fn_with_prefix;

#[cfg(test)]
mod tests;
