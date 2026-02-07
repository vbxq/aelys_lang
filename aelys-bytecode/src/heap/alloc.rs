use super::Heap;
use crate::Function;
use crate::object::{AelysFunction, AelysString, GcObject, GcRef, NativeFunction, ObjectKind};

impl Heap {
    pub fn alloc(&mut self, obj: GcObject) -> GcRef {
        self.bytes_allocated += Self::estimate_object_size(&obj);

        // reuse free slots when possible
        if let Some(idx) = self.free_list.pop() {
            self.objects[idx] = Some(obj);
            GcRef::new(idx)
        } else {
            let idx = self.objects.len();
            self.objects.push(Some(obj));
            GcRef::new(idx)
        }
    }

    pub fn alloc_string(&mut self, s: &str) -> GcRef {
        self.alloc(GcObject::new(ObjectKind::String(AelysString::new(s))))
    }

    pub fn alloc_function(&mut self, func: Function) -> GcRef {
        self.alloc(GcObject::new(ObjectKind::Function(AelysFunction::new(
            func,
        ))))
    }

    pub fn alloc_native(&mut self, name: &str, arity: u8) -> GcRef {
        self.alloc(GcObject::new(ObjectKind::Native(NativeFunction::new(
            name, arity,
        ))))
    }

    // same as alloc_native, just different name for clarity in calling code
    pub fn alloc_foreign(&mut self, name: &str, arity: u8) -> GcRef {
        self.alloc_native(name, arity)
    }
}
