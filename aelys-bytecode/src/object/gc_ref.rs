/// Reference to a GC object (index into the heap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GcRef(usize);

impl GcRef {
    pub fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn index(&self) -> usize {
        self.0
    }
}

impl From<usize> for GcRef {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl From<GcRef> for usize {
    fn from(gc_ref: GcRef) -> Self {
        gc_ref.0
    }
}
