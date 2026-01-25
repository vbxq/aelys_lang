use super::super::Value;

/// Error type for manual heap operations.
#[derive(Debug, Clone)]
pub enum ManualHeapError {
    /// Allocation size must be > 0.
    InvalidSize,
    /// Handle does not refer to a valid allocation.
    InvalidHandle,
    /// Pointer was already freed.
    DoubleFree {
        #[cfg(debug_assertions)]
        alloc_line: u32,
    },
    /// Attempt to access freed memory.
    UseAfterFree {
        #[cfg(debug_assertions)]
        freed_line: u32,
    },
    /// Offset exceeds allocation size.
    OutOfBounds { offset: usize, size: usize },
}

pub(super) fn allocation_bytes(size: usize) -> Result<usize, ManualHeapError> {
    size.checked_mul(std::mem::size_of::<Value>())
        .ok_or(ManualHeapError::InvalidSize)
}
