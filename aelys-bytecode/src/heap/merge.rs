use super::Heap;
use crate::object::GcRef;

impl Heap {
    /// Merge another heap into this heap.
    pub fn merge(&mut self, other: &mut Heap) -> std::collections::HashMap<usize, usize> {
        let mut remap = std::collections::HashMap::new();

        for (old_idx, slot) in other.objects.iter_mut().enumerate() {
            if let Some(obj) = slot.take() {
                let new_ref = self.alloc(obj);
                remap.insert(old_idx, new_ref.index());
            }
        }

        for (hash, old_ref) in other.intern_table.drain() {
            if let Some(&new_idx) = remap.get(&old_ref.index()) {
                self.intern_table.entry(hash).or_insert(GcRef::new(new_idx));
            }
        }

        other.objects.clear();
        other.free_list.clear();
        other.bytes_allocated = 0;

        remap
    }
}
