use super::Heap;
use crate::object::{GcObject, GcRef, ObjectKind};

impl Heap {
    pub fn get(&self, gc_ref: GcRef) -> Option<&GcObject> {
        self.objects.get(gc_ref.index())?.as_ref()
    }

    /// Get a GC object without bounds checking.
    ///
    /// # Safety
    /// - The caller must guarantee that `gc_ref.index()` is within bounds of `self.objects`.
    /// - The slot at `gc_ref.index()` must contain a valid object (not None).
    // TODO: Future optimization for faster heap access in hot paths.
    #[allow(dead_code)]
    #[inline(always)]
    pub unsafe fn get_unchecked(&self, gc_ref: GcRef) -> &GcObject {
        unsafe {
            self.objects
                .get_unchecked(gc_ref.index())
                .as_ref()
                .unwrap_unchecked()
        }
    }

    pub fn get_mut(&mut self, gc_ref: GcRef) -> Option<&mut GcObject> {
        self.objects.get_mut(gc_ref.index())?.as_mut()
    }

    pub fn get_type_name(&self, gc_ref: GcRef) -> &'static str {
        if let Some(obj) = self.get(gc_ref) {
            match &obj.kind {
                ObjectKind::String(_) => "String",
                ObjectKind::Function(_) => "Function",
                ObjectKind::Native(_) => "NativeFunction",
                ObjectKind::Upvalue(_) => "Upvalue",
                ObjectKind::Closure(_) => "Closure",
                ObjectKind::Array(_) => "Array",
                ObjectKind::Vec(_) => "Vec",
            }
        } else {
            "Unknown"
        }
    }

    pub fn should_collect(&self) -> bool {
        self.bytes_allocated >= self.next_gc
    }

    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated
    }

    pub fn next_gc_threshold(&self) -> usize {
        self.next_gc
    }

    pub fn object_count(&self) -> usize {
        self.objects.iter().filter(|o| o.is_some()).count()
    }
}
