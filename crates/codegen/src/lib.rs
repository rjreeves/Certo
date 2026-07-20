mod ty_to_c;
mod emit_mir;
mod emit_module;
pub mod emit_validator;
pub mod emit_statemachine;

pub use emit_module::{emit_module, CodegenOptions, RUNTIME_HEADER};
pub use emit_mir::{emit_fn_with_prefix, c_fn_name};
pub use emit_validator::{emit_validator, ValidatorOutput};
pub use emit_statemachine::emit_state_machine;

#[cfg(test)]
mod tests;
