use std::cell::UnsafeCell;
use std::sync::Arc;

// Bytecode buffer with interior mutability for inline cache patching.
// Arc for shared ownership, UnsafeCell for patching without &mut.
// SAFETY: VM is single-threaded, patching happens during execution only.
// Would need AtomicU32 if we ever go multi-threaded
#[derive(Clone)]
pub struct BytecodeBuffer(Arc<UnsafeCell<Box<[u32]>>>);

impl std::fmt::Debug for BytecodeBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BytecodeBuffer")
            .field(&format!("[{} words]", self.len()))
            .finish()
    }
}

impl BytecodeBuffer {
    pub fn new(data: Box<[u32]>) -> Self { Self(Arc::new(UnsafeCell::new(data))) }
    pub fn empty() -> Self { Self::new(Box::new([])) }
    pub fn from_vec(v: Vec<u32>) -> Self { Self::new(v.into_boxed_slice()) }

    #[inline(always)]
    pub fn len(&self) -> usize { unsafe { (&*self.0.get()).len() } }

    #[inline(always)]
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    // raw pointers for dispatch loop - valid as long as buffer lives
    #[inline(always)]
    pub fn as_ptr(&self) -> *const u32 { unsafe { (&*self.0.get()).as_ptr() } }

    #[inline(always)]
    pub fn as_mut_ptr(&self) -> *mut u32 { unsafe { (&mut *self.0.get()).as_mut_ptr() } }

    #[inline(always)]
    pub fn read(&self, off: usize) -> u32 { unsafe { *(&*self.0.get()).get_unchecked(off) } }

    // for inline cache patching
    #[inline(always)]
    pub fn patch(&self, off: usize, val: u32) {
        unsafe { *(&mut *self.0.get()).get_unchecked_mut(off) = val; }
    }

    pub fn as_slice(&self) -> &[u32] { unsafe { &**self.0.get() } }

    pub fn iter(&self) -> impl Iterator<Item = &u32> { self.as_slice().iter() }
}

impl std::ops::Index<usize> for BytecodeBuffer {
    type Output = u32;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

impl PartialEq for BytecodeBuffer {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl From<Vec<u32>> for BytecodeBuffer {
    fn from(v: Vec<u32>) -> Self {
        Self::from_vec(v)
    }
}

impl From<Arc<[u32]>> for BytecodeBuffer {
    fn from(arc: Arc<[u32]>) -> Self {
        Self::new(arc.to_vec().into_boxed_slice())
    }
}

unsafe impl Send for BytecodeBuffer {}
unsafe impl Sync for BytecodeBuffer {}
