mod mir;
mod lower;

pub use mir::*;
pub use lower::lower_fn;

#[cfg(test)]
mod tests;
