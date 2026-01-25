use crate::vm::{Function, Heap};

mod bytecode;
mod checks;
mod constants;

pub const MAX_FUNCTION_NESTING: usize = 64;

pub fn verify_function(func: &Function, heap: &Heap, depth: usize) -> Result<(), String> {
    if depth > MAX_FUNCTION_NESTING {
        return Err(format!(
            "nesting depth {} exceeds max {}",
            depth, MAX_FUNCTION_NESTING
        ));
    }

    constants::verify_constants(func, heap)?;
    bytecode::verify_bytecode(func)?;

    for nested in &func.nested_functions {
        verify_function(nested, heap, depth + 1)?;
    }

    Ok(())
}
