use super::super::Value;
use super::error::ManualHeapError;
use super::error::allocation_bytes;
use super::heap::{ManualAllocation, ManualHeap};

impl ManualHeap {
    /// Allocate `size` slots of memory.
    ///
    /// Returns a handle (index) that can be used with load/store/free.
    /// All slots are initialized to null.
    #[allow(unused_variables)]
    pub fn alloc(&mut self, size: usize, line: u32) -> Result<usize, ManualHeapError> {
        if size == 0 {
            return Err(ManualHeapError::InvalidSize);
        }

        let bytes = allocation_bytes(size)?;
        let allocation = ManualAllocation {
            data: vec![Value::null(); size],
            freed: false,
            #[cfg(debug_assertions)]
            alloc_line: line,
            #[cfg(debug_assertions)]
            freed_line: 0,
        };

        let handle = if let Some(idx) = self.free_list.pop() {
            self.allocations[idx] = allocation;
            idx
        } else {
            let idx = self.allocations.len();
            self.allocations.push(allocation);
            idx
        };

        self.bytes_allocated = self
            .bytes_allocated
            .checked_add(bytes)
            .ok_or(ManualHeapError::InvalidSize)?;

        Ok(handle)
    }

    /// Free a previously allocated block.
    ///
    /// The handle becomes invalid after this call.
    pub fn free(&mut self, handle: usize, line: u32) -> Result<(), ManualHeapError> {
        let alloc = self
            .allocations
            .get_mut(handle)
            .ok_or(ManualHeapError::InvalidHandle)?;

        if alloc.freed {
            return Err(ManualHeapError::DoubleFree {
                #[cfg(debug_assertions)]
                alloc_line: alloc.alloc_line,
            });
        }

        alloc.freed = true;
        #[cfg(debug_assertions)]
        {
            alloc.freed_line = line;
        }

        let bytes = alloc
            .data
            .len()
            .checked_mul(std::mem::size_of::<Value>())
            .unwrap_or(0);
        let _ = line;
        alloc.data.clear();
        alloc.data.shrink_to_fit();
        self.free_list.push(handle);
        self.bytes_allocated = self.bytes_allocated.saturating_sub(bytes);
        Ok(())
    }
}
