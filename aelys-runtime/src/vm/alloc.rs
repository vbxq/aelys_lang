use super::manual_heap::ManualHeapError;
use super::{
    AelysFunction, GcObject, GcRef, Heap, NativeFn, NativeFunction, NativeFunctionImpl, ObjectKind,
    VM, Value,
};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use aelys_native::AelysNativeFn;

impl VM {
    fn ensure_heap_capacity(&self, additional: u64) -> Result<(), RuntimeError> {
        let heap_bytes = self.heap.bytes_allocated() as u64;
        let manual_bytes = self.manual_heap.bytes_allocated() as u64;
        let used = heap_bytes.checked_add(manual_bytes).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::OutOfMemory {
                requested: additional,
                max: self.config.max_heap_bytes,
            })
        })?;
        let new_total = used.checked_add(additional).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::OutOfMemory {
                requested: additional,
                max: self.config.max_heap_bytes,
            })
        })?;
        if new_total > self.config.max_heap_bytes {
            return Err(self.runtime_error(RuntimeErrorKind::OutOfMemory {
                requested: additional,
                max: self.config.max_heap_bytes,
            }));
        }
        Ok(())
    }

    pub fn alloc_object(&mut self, object: GcObject) -> Result<GcRef, RuntimeError> {
        let size = Heap::estimate_object_size(&object) as u64;
        self.ensure_heap_capacity(size)?;
        Ok(self.heap.alloc(object))
    }

    pub fn alloc_string(&mut self, s: &str) -> Result<GcRef, RuntimeError> {
        let size = Heap::estimate_string_size(s.len()) as u64;
        self.ensure_heap_capacity(size)?;
        Ok(self.heap.alloc_string(s))
    }

    pub fn intern_string(&mut self, s: &str) -> Result<GcRef, RuntimeError> {
        if let Some(existing) = self.heap.find_interned_string(s) {
            return Ok(existing);
        }
        let size = Heap::estimate_string_size(s.len()) as u64;
        self.ensure_heap_capacity(size)?;
        Ok(self.heap.intern_string(s))
    }

    pub fn alloc_function(&mut self, func: super::Function) -> Result<GcRef, RuntimeError> {
        let obj = GcObject::new(ObjectKind::Function(AelysFunction::new(func)));
        self.alloc_object(obj)
    }

    pub fn alloc_native(
        &mut self,
        name: &str,
        arity: u8,
        func: NativeFn,
    ) -> Result<GcRef, RuntimeError> {
        self.native_registry
            .insert(name.to_string(), NativeFunctionImpl::Rust(func));
        let obj = GcObject::new(ObjectKind::Native(NativeFunction::new(name, arity)));
        self.alloc_object(obj)
    }

    pub fn alloc_foreign(
        &mut self,
        name: &str,
        arity: u8,
        func: AelysNativeFn,
    ) -> Result<GcRef, RuntimeError> {
        self.native_registry
            .insert(name.to_string(), NativeFunctionImpl::Foreign(func));
        let obj = GcObject::new(ObjectKind::Native(NativeFunction::new(name, arity)));
        self.alloc_object(obj)
    }

    pub fn manual_alloc(&mut self, size: usize, line: u32) -> Result<usize, RuntimeError> {
        let bytes = (size as u64)
            .checked_mul(std::mem::size_of::<Value>() as u64)
            .ok_or_else(|| {
                self.runtime_error(RuntimeErrorKind::OutOfMemory {
                    requested: self.config.max_heap_bytes.saturating_add(1),
                    max: self.config.max_heap_bytes,
                })
            })?;
        self.ensure_heap_capacity(bytes)?;
        self.manual_heap
            .alloc(size, line)
            .map_err(|e| self.manual_heap_error(e))
    }

    pub fn manual_free(&mut self, handle: usize, line: u32) -> Result<(), RuntimeError> {
        self.manual_heap
            .free(handle, line)
            .map_err(|e| self.manual_heap_error(e))
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    pub fn heap_mut(&mut self) -> &mut Heap {
        &mut self.heap
    }

    pub fn manual_heap(&self) -> &super::ManualHeap {
        &self.manual_heap
    }

    pub fn manual_heap_mut(&mut self) -> &mut super::ManualHeap {
        &mut self.manual_heap
    }

    pub fn merge_heap(
        &mut self,
        compile_heap: &mut Heap,
    ) -> Result<std::collections::HashMap<usize, usize>, RuntimeError> {
        let added_bytes = compile_heap.bytes_allocated() as u64;
        self.ensure_heap_capacity(added_bytes)?;
        Ok(self.heap.merge(compile_heap))
    }

    pub fn manual_heap_error(&self, err: ManualHeapError) -> RuntimeError {
        let kind = match err {
            ManualHeapError::InvalidSize => RuntimeErrorKind::InvalidAllocationSize { size: 0 },
            ManualHeapError::InvalidHandle => RuntimeErrorKind::InvalidMemoryHandle,
            ManualHeapError::DoubleFree { .. } => RuntimeErrorKind::DoubleFree,
            ManualHeapError::UseAfterFree { .. } => RuntimeErrorKind::UseAfterFree,
            ManualHeapError::OutOfBounds { offset, size } => {
                RuntimeErrorKind::MemoryOutOfBounds { offset, size }
            }
        };
        self.runtime_error(kind)
    }
}
