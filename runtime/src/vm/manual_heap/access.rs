use super::super::Value;
use super::error::ManualHeapError;
use super::heap::ManualHeap;

impl ManualHeap {
    /// Load a value from the given offset in an allocation.
    pub fn load(&self, handle: usize, offset: usize) -> Result<Value, ManualHeapError> {
        let alloc = self
            .allocations
            .get(handle)
            .ok_or(ManualHeapError::InvalidHandle)?;

        if alloc.freed {
            return Err(ManualHeapError::UseAfterFree {
                #[cfg(debug_assertions)]
                freed_line: alloc.freed_line,
            });
        }

        alloc
            .data
            .get(offset)
            .copied()
            .ok_or(ManualHeapError::OutOfBounds {
                offset,
                size: alloc.data.len(),
            })
    }

    /// Load a value without any bounds or validity checking.
    ///
    /// # Safety
    /// The caller must ensure that `handle` is a valid allocation index and
    /// `offset` is within the allocation's bounds.
    #[inline(always)]
    pub unsafe fn load_unchecked(&self, handle: usize, offset: usize) -> Value {
        unsafe {
            *self
                .allocations
                .get_unchecked(handle)
                .data
                .get_unchecked(offset)
        }
    }

    /// Store a value at the given offset in an allocation.
    pub fn store(
        &mut self,
        handle: usize,
        offset: usize,
        value: Value,
    ) -> Result<(), ManualHeapError> {
        let alloc = self
            .allocations
            .get_mut(handle)
            .ok_or(ManualHeapError::InvalidHandle)?;

        if alloc.freed {
            return Err(ManualHeapError::UseAfterFree {
                #[cfg(debug_assertions)]
                freed_line: alloc.freed_line,
            });
        }

        if offset >= alloc.data.len() {
            return Err(ManualHeapError::OutOfBounds {
                offset,
                size: alloc.data.len(),
            });
        }

        alloc.data[offset] = value;
        Ok(())
    }

    /// Store a value without any bounds or validity checking.
    ///
    /// # Safety
    /// The caller must ensure that `handle` is a valid allocation index and
    /// `offset` is within the allocation's bounds.
    #[inline(always)]
    pub unsafe fn store_unchecked(&mut self, handle: usize, offset: usize, value: Value) {
        unsafe {
            *self
                .allocations
                .get_unchecked_mut(handle)
                .data
                .get_unchecked_mut(offset) = value;
        }
    }

    /// Get the size of an allocation.
    pub fn size(&self, handle: usize) -> Result<usize, ManualHeapError> {
        let alloc = self
            .allocations
            .get(handle)
            .ok_or(ManualHeapError::InvalidHandle)?;

        if alloc.freed {
            return Err(ManualHeapError::UseAfterFree {
                #[cfg(debug_assertions)]
                freed_line: alloc.freed_line,
            });
        }

        Ok(alloc.data.len())
    }
}
