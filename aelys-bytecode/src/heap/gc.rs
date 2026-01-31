use super::Heap;
use crate::object::{AelysClosure, AelysString, AelysUpvalue, GcRef, ObjectKind};
use crate::value::Value;

impl Heap {
    // mark-sweep GC, nothing fancy. worklist avoids recursion (stack overflow on deep graphs)
    pub fn mark(&mut self, root: GcRef) {
        let mut worklist = vec![root];

        while let Some(r) = worklist.pop() {
            let should_trace = self.get_mut(r).map(|o| {
                if o.marked { false } else { o.marked = true; true }
            }).unwrap_or(false);

            if should_trace {
                if let Some(obj) = self.get(r) {
                    match &obj.kind {
                        ObjectKind::Function(f) => {
                            for v in &f.function.constants {
                                if let Some(p) = v.as_ptr() { worklist.push(GcRef::new(p)); }
                            }
                        }
                        ObjectKind::Closure(c) => {
                            worklist.push(c.function);
                            worklist.extend(c.upvalues.iter().cloned());
                        }
                        ObjectKind::Upvalue(u) => {
                            if let crate::object::UpvalueLocation::Closed(v) = &u.location {
                                if let Some(p) = v.as_ptr() { worklist.push(GcRef::new(p)); }
                            }
                        }
                        ObjectKind::String(_) | ObjectKind::Native(_) => {}
                        ObjectKind::Array(a) => {
                            if let Some(objs) = a.data.as_objects() {
                                for v in objs {
                                    if let Some(p) = v.as_ptr() { worklist.push(GcRef::new(p)); }
                                }
                            }
                        }
                        ObjectKind::Vec(vec) => {
                            if let Some(objs) = vec.objects() {
                                for v in objs {
                                    if let Some(p) = v.as_ptr() { worklist.push(GcRef::new(p)); }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn sweep(&mut self) -> usize {
        let mut freed = 0;

        for (idx, slot) in self.objects.iter_mut().enumerate() {
            let should_free = slot.as_ref().map(|o| !o.marked).unwrap_or(false);
            if let Some(obj) = slot.as_mut() { obj.marked = false; }

            if should_free {
                if let Some(obj) = slot.take() {
                    self.bytes_allocated = self.bytes_allocated.saturating_sub(Self::estimate_object_size(&obj));
                    if let ObjectKind::String(s) = &obj.kind { self.intern_table.remove(&s.hash()); }
                    self.free_list.push(idx);
                    freed += 1;
                }
            }
        }

        // grow threshold after collection
        self.next_gc = (self.bytes_allocated * Self::GC_GROWTH_FACTOR).max(Self::INITIAL_GC_THRESHOLD);
        freed
    }

    pub fn estimate_object_size(obj: &crate::object::GcObject) -> usize {
        match &obj.kind {
            ObjectKind::String(s) => std::mem::size_of::<AelysString>() + s.len(),
            ObjectKind::Function(f) => {
                std::mem::size_of::<crate::object::AelysFunction>()
                    + f.function.bytecode.len() * 4
                    + f.function.constants.len() * std::mem::size_of::<Value>()
            }
            ObjectKind::Native(_) => std::mem::size_of::<crate::object::NativeFunction>(),
            ObjectKind::Upvalue(_) => std::mem::size_of::<AelysUpvalue>(),
            ObjectKind::Closure(c) => std::mem::size_of::<AelysClosure>() + c.upvalues.len() * 8,
            ObjectKind::Array(a) => a.size_bytes(),
            ObjectKind::Vec(v) => v.size_bytes(),
        }
    }
}
