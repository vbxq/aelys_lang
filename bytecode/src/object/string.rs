use std::hash::{Hash, Hasher};

// immutable string with cached hash (for fast comparison and interning)
#[derive(Debug, Clone)]
pub struct AelysString {
    hash: u64,
    data: Box<[u8]>,
}

impl AelysString {
    pub fn new(s: &str) -> Self {
        let data = s.as_bytes().to_vec().into_boxed_slice();
        Self {
            hash: Self::compute_hash(&data),
            data,
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let hash = Self::compute_hash(&bytes);
        Self {
            hash,
            data: bytes.into_boxed_slice(),
        }
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    // SAFETY: we only store valid utf-8
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.data) }
    }

    fn compute_hash(bytes: &[u8]) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        h.finish()
    }
}

impl PartialEq for AelysString {
    fn eq(&self, other: &Self) -> bool {
        if self.hash != other.hash {
            return false;
        }
        self.data == other.data
    }
}

impl Eq for AelysString {}

impl Hash for AelysString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state);
    }
}
