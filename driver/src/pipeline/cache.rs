use super::types::StageOutput;
use aelys_syntax::Source;
use std::hash::{Hash, Hasher};

#[derive(Clone)]
pub(crate) struct CachedOutput {
    pub(crate) source_hash: u64,
    pub(crate) output: StageOutput,
}

pub(crate) fn source_hash(source: &Source) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    source.name.hash(&mut hasher);
    source.content.hash(&mut hasher);
    hasher.finish()
}
