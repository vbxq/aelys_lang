use super::super::{ObjectKind, VM};
use std::sync::Arc;

impl VM {
    /// Synchronize indexed globals back to the hash map.
    pub fn sync_globals_to_hashmap(&mut self, global_names: &[String]) {
        for (idx, name) in global_names.iter().enumerate() {
            if !name.is_empty() && idx < self.globals_by_index.len() {
                let value = self.globals_by_index[idx];
                if !value.is_null() || !self.globals.contains_key(name) {
                    self.globals.insert(name.clone(), value);
                }
            }
        }
    }

    /// Sync the current function's globals_by_index to the globals hashmap.
    pub fn sync_current_function_globals(&mut self) {
        if let Some(frame) = self.frames.last() {
            let func_ref = frame.function;
            let global_layout: Option<Arc<super::super::GlobalLayout>> = {
                if let Some(obj) = self.heap.get(func_ref) {
                    match &obj.kind {
                        ObjectKind::Function(f) => {
                            if !f.function.global_layout.names().is_empty() {
                                Some(Arc::clone(&f.function.global_layout))
                            } else {
                                None
                            }
                        }
                        ObjectKind::Closure(c) => {
                            if let Some(inner_obj) = self.heap.get(c.function) {
                                if let ObjectKind::Function(f) = &inner_obj.kind {
                                    if !f.function.global_layout.names().is_empty() {
                                        Some(Arc::clone(&f.function.global_layout))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            };

            if let Some(layout) = global_layout {
                for (idx, name) in layout.names().iter().enumerate() {
                    if !name.is_empty() && idx < self.globals_by_index.len() {
                        let value = self.globals_by_index[idx];
                        if !value.is_null() {
                            self.globals.insert(name.clone(), value);
                        }
                    }
                }
            }
        }
    }
}
