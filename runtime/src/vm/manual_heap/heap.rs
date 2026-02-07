use super::super::Value;
use super::error::ManualHeapError;
use super::guard::ManualHeapGuard;

/// A single manual allocation.
pub(super) struct ManualAllocation {
    pub(super) data: Vec<Value>,
    pub(super) freed: bool,
    #[cfg(debug_assertions)]
    pub(super) alloc_line: u32,
    #[cfg(debug_assertions)]
    pub(super) freed_line: u32,
}

/// The manual memory heap.
///
/// Allocations persist independently of GC and @no_gc scope.
/// Developer is responsible for calling free().
pub struct ManualHeap {
    pub(super) allocations: Vec<ManualAllocation>,
    pub(super) free_list: Vec<usize>,
    pub(super) bytes_allocated: usize,
}

impl ManualHeap {
    /// Create a new empty manual heap.
    pub fn new() -> Self {
        Self {
            allocations: Vec::new(),
            free_list: Vec::new(),
            bytes_allocated: 0,
        }
    }

    pub fn bytes_allocated(&self) -> usize {
        self.bytes_allocated
    }

    /// Allocate with a guard that warns if not explicitly freed.
    pub fn alloc_guarded(
        &mut self,
        size: usize,
        line: u32,
    ) -> Result<ManualHeapGuard, ManualHeapError> {
        let handle = self.alloc(size, line)?;
        Ok(ManualHeapGuard::new(handle, line))
    }

    /// Scoped allocation with guaranteed cleanup via RAII.
    pub fn with_allocation<F, R>(
        &mut self,
        size: usize,
        line: u32,
        f: F,
    ) -> Result<R, ManualHeapError>
    where
        F: FnOnce(usize, &mut Self) -> Result<R, ManualHeapError>,
    {
        let handle = self.alloc(size, line)?;
        let result = f(handle, self);
        self.free(handle, line)?;
        result
    }
}

impl Default for ManualHeap {
    fn default() -> Self {
        Self::new()
    }
}
