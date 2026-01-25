use super::Heap;
use crate::object::{AelysString, GcObject, GcRef, ObjectKind};

impl Heap {
    // lookup only, doesn't insert
    pub fn find_interned_string(&self, s: &str) -> Option<GcRef> {
        let r = *self.intern_table.get(&Self::fnv1a_hash(s.as_bytes()))?;
        match self.get(r)?.kind {
            ObjectKind::String(ref existing) if existing.as_str() == s => Some(r),
            _ => None,
        }
    }

    // intern or return existing
    pub fn intern_string(&mut self, s: &str) -> GcRef {
        let hash = Self::fnv1a_hash(s.as_bytes());

        // check if already interned
        if let Some(&r) = self.intern_table.get(&hash) {
            if let Some(obj) = self.get(r) {
                if let ObjectKind::String(ref existing) = obj.kind {
                    if existing.as_str() == s { return r; }
                }
            }
        }

        let r = self.alloc(GcObject::new(ObjectKind::String(AelysString::new(s))));
        self.intern_table.insert(hash, r);
        r
    }

    // FNV-1a - simple and fast enough for string interning
    pub fn fnv1a_hash(bytes: &[u8]) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for &b in bytes { h = (h ^ b as u64).wrapping_mul(0x100000001b3); }
        h
    }
}
