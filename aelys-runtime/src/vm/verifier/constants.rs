use crate::vm::GcRef;
use crate::vm::{Function, Heap};

pub(super) fn verify_constants(func: &Function, heap: &Heap) -> Result<(), String> {
    for (idx, value) in func.constants.iter().enumerate() {
        if let Some(ptr) = value.as_ptr() {
            // Nested function marker.
            if (ptr & (1 << 20)) != 0 {
                let func_idx = ptr & 0xFFFFF;
                if func_idx >= func.nested_functions.len() {
                    return Err(format!(
                        "constant {} has invalid nested function index {}",
                        idx, func_idx
                    ));
                }
                continue;
            }

            if heap.get(GcRef::new(ptr)).is_none() {
                return Err(format!(
                    "constant {} has invalid heap reference {}",
                    idx, ptr
                ));
            }
        }
    }
    Ok(())
}
