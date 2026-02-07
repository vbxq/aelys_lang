// mark-and-sweep, scans only num_registers per frame
// TODO: incremental/generational GC would be nice for larger heaps

use super::{GcRef, MAX_NO_GC_DEPTH, VM};

impl VM {
    pub fn enter_no_gc(&mut self) {
        if self.no_gc_depth >= MAX_NO_GC_DEPTH {
            return;
        }
        self.no_gc_depth += 1;
    }

    pub fn exit_no_gc(&mut self) {
        if self.no_gc_depth == 0 {
            return;
        }
        self.no_gc_depth -= 1;
    }

    pub fn is_in_no_gc(&self) -> bool {
        self.no_gc_depth > 0
    }

    pub fn no_gc_depth(&self) -> usize {
        self.no_gc_depth
    }

    pub fn maybe_collect(&mut self) {
        if self.is_in_no_gc() {
            return;
        }
        if self.heap.should_collect() {
            self.collect();
        }
    }

    pub fn collect(&mut self) {
        for frame in &self.frames {
            let base = frame.base;
            let count = frame.num_registers as usize;
            for i in 0..count {
                let idx = base + i;
                if idx < self.registers.len()
                    && let Some(gc_ref) = self.registers[idx].as_ptr()
                {
                    self.heap.mark(GcRef::new(gc_ref));
                }
            }
            self.heap.mark(frame.function());
        }

        for value in self.globals.values() {
            if let Some(gc_ref) = value.as_ptr() {
                self.heap.mark(GcRef::new(gc_ref));
            }
        }

        for value in &self.globals_by_index {
            if let Some(gc_ref) = value.as_ptr() {
                self.heap.mark(GcRef::new(gc_ref));
            }
        }

        for &upval_ref in &self.open_upvalues {
            self.heap.mark(upval_ref);
        }
        for &upval_ref in &self.current_upvalues {
            self.heap.mark(upval_ref);
        }

        self.heap.sweep();
        // call_site_cache is not cleared here; it's invalidated on global mutation
        // (set_global/set_global_by_index), which prevents use-after-free.
        self.globals_by_index_cache.clear();
    }
}
