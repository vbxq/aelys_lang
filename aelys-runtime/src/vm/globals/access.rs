use super::super::{VM, Value};

impl VM {
    pub fn get_global(&self, name: &str) -> Option<Value> {
        self.globals.get(name).copied()
    }

    pub fn set_global(&mut self, name: String, value: Value) {
        self.globals.insert(name, value);
        self.globals_by_index_cache.clear();
    }

    pub fn set_global_by_index(&mut self, idx: usize, value: Value) {
        if idx >= self.globals_by_index.len() {
            self.globals_by_index.resize(idx + 1, Value::null());
        }
        self.globals_by_index[idx] = value;
        self.globals_by_index_cache.clear();
    }

    pub fn global_mutability(&self) -> &std::collections::HashMap<String, bool> {
        &self.global_mutability
    }

    pub fn update_global_mutability(
        &mut self,
        new_globals: std::collections::HashMap<String, bool>,
    ) {
        self.global_mutability.extend(new_globals);
    }
}
