use super::ManualHeap;
use super::ManualHeapError;

/// RAII guard for manual heap allocations.
/// The guard does NOT auto-free because it cannot hold a mutable reference
/// to the heap (Rust borrowing rules). Instead, you must explicitly call free()
/// before the guard is dropped.
pub struct ManualHeapGuard {
    handle: usize,
    freed: bool,
    #[cfg(debug_assertions)]
    alloc_line: u32,
}

impl ManualHeapGuard {
    /// Create a new guard for an allocation handle.
    pub(super) fn new(handle: usize, #[allow(unused)] line: u32) -> Self {
        Self {
            handle,
            freed: false,
            #[cfg(debug_assertions)]
            alloc_line: line,
        }
    }

    /// Get the handle (for passing to load/store operations).
    #[inline]
    pub fn handle(&self) -> usize {
        self.handle
    }

    /// Free the allocation explicitly.
    ///
    /// This marks the guard as freed and calls the heap's free method.
    pub fn free(mut self, heap: &mut ManualHeap, line: u32) -> Result<(), ManualHeapError> {
        self.freed = true;
        heap.free(self.handle, line)
    }

    /// Consume the guard without freeing (transfer ownership).
    ///
    /// Use this when you need to pass the handle to code that will
    /// manage its lifetime separately.
    pub fn leak(mut self) -> usize {
        self.freed = true;
        let handle = self.handle;
        std::mem::forget(self);
        handle
    }
}

impl Drop for ManualHeapGuard {
    fn drop(&mut self) {
        if !self.freed {
            #[cfg(debug_assertions)]
            eprintln!(
                "WARNING: ManualHeapGuard dropped without freeing handle {} (allocated at line {})",
                self.handle, self.alloc_line
            );
            #[cfg(not(debug_assertions))]
            eprintln!(
                "WARNING: ManualHeapGuard dropped without freeing handle {}",
                self.handle
            );
        }
    }
}
