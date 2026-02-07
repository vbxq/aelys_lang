use super::super::{GcRef, ObjectKind, VM, Value};
use std::sync::Arc;

impl VM {
    /// Get the global mapping ID for a function (global layout ID).
    pub fn get_global_mapping_id(&self, func_ref: GcRef) -> usize {
        if let Some(obj) = self.heap.get(func_ref) {
            match &obj.kind {
                ObjectKind::Function(f) => f.function.global_layout.id(),
                ObjectKind::Closure(c) => {
                    if let Some(inner_obj) = self.heap.get(c.function) {
                        if let ObjectKind::Function(f) = &inner_obj.kind {
                            f.function.global_layout.id()
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                }
                _ => 0,
            }
        } else {
            0
        }
    }

    pub(crate) fn global_mapping_id_for_layout(
        &self,
        layout: &super::super::GlobalLayout,
    ) -> usize {
        layout.id()
    }

    /// Prepare globals_by_index for a function call.
    pub fn prepare_globals_for_function(&mut self, func_ref: GcRef) -> usize {
        let (mapping_id, global_layout): (usize, Option<Arc<super::super::GlobalLayout>>) = {
            if let Some(obj) = self.heap.get(func_ref) {
                match &obj.kind {
                    ObjectKind::Function(f) => {
                        let layout = Arc::clone(&f.function.global_layout);
                        let id = self.global_mapping_id_for_layout(&layout);
                        if layout.names().is_empty() {
                            (id, None)
                        } else {
                            (id, Some(layout))
                        }
                    }
                    ObjectKind::Closure(c) => {
                        if let Some(inner_obj) = self.heap.get(c.function) {
                            if let ObjectKind::Function(f) = &inner_obj.kind {
                                let layout = Arc::clone(&f.function.global_layout);
                                let id = self.global_mapping_id_for_layout(&layout);
                                if layout.names().is_empty() {
                                    (id, None)
                                } else {
                                    (id, Some(layout))
                                }
                            } else {
                                (0, None)
                            }
                        } else {
                            (0, None)
                        }
                    }
                    _ => (0, None),
                }
            } else {
                (0, None)
            }
        };

        if mapping_id == self.current_global_mapping_id {
            return mapping_id;
        }

        if global_layout.is_none() {
            self.globals_by_index.clear();
            self.current_global_mapping_id = mapping_id;
            self.globals_by_index_cache
                .insert(mapping_id, Arc::new(Vec::new()));
            return mapping_id;
        }

        if let Some(cached) = self.globals_by_index_cache.get(&mapping_id) {
            let len = cached.len();
            self.globals_by_index.clear();
            self.globals_by_index.reserve_exact(len);
            unsafe {
                self.globals_by_index.set_len(len);
                std::ptr::copy_nonoverlapping(
                    cached.as_ptr(),
                    self.globals_by_index.as_mut_ptr(),
                    len,
                );
            }
            self.current_global_mapping_id = mapping_id;
            return mapping_id;
        }

        let layout = match global_layout.as_ref() {
            Some(layout) => layout,
            None => {
                self.current_global_mapping_id = mapping_id;
                return mapping_id;
            }
        };
        let names = layout.names();

        let needed_len = names.len();
        if self.globals_by_index.len() < needed_len {
            self.globals_by_index.resize(needed_len, Value::null());
        }
        for (idx, name) in names.iter().enumerate() {
            if !name.is_empty() {
                self.globals_by_index[idx] =
                    self.globals.get(name).copied().unwrap_or(Value::null());
            } else {
                self.globals_by_index[idx] = Value::null();
            }
        }
        self.globals_by_index.truncate(needed_len);

        self.current_global_mapping_id = mapping_id;
        self.globals_by_index_cache
            .insert(mapping_id, Arc::new(self.globals_by_index.clone()));
        mapping_id
    }
}
