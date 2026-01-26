use super::Function;
use crate::value::Value;
use std::collections::HashMap;

impl Function {
    /// Compute and set the global_layout_hash from global layout names
    pub fn compute_global_layout_hash(&mut self) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        if self.global_layout.names().is_empty() {
            self.global_layout_hash = 0;
        } else {
            let mut hasher = DefaultHasher::new();
            self.global_layout.names().hash(&mut hasher);
            self.global_layout_hash = hasher.finish() | 1;
        }
    }

    /// Add a constant and return its index
    pub fn add_constant(&mut self, value: Value) -> u16 {
        for (i, existing) in self.constants.iter().enumerate() {
            if *existing == value {
                return i as u16;
            }
        }

        let idx = self.constants.len() as u16;
        self.constants.push(value);
        idx
    }

    /// Add a nested function and return a special constant index for it
    pub fn add_constant_function(&mut self, func: Function) -> u16 {
        let func_idx = self.nested_functions.len();
        self.nested_functions.push(func);

        // Use dedicated tag to avoid collision with heap pointers
        let marker = Value::nested_fn_marker(func_idx);

        let idx = self.constants.len() as u16;
        self.constants.push(marker);
        idx
    }

    /// Remap GcRef pointers in constants using the provided mapping.
    pub fn remap_constants(&mut self, remap: &HashMap<usize, usize>) {
        for constant in &mut self.constants {
            if let Some(old_idx) = constant.as_ptr() {
                if let Some(&new_idx) = remap.get(&old_idx) {
                    *constant = Value::ptr(new_idx);
                }
            }
        }

        for nested_func in &mut self.nested_functions {
            nested_func.remap_constants(remap);
        }
    }
}
