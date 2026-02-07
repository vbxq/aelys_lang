// gc heap for bytecode constants and runtime objects

mod access;
mod alloc;
mod gc;
mod merge;
mod strings;

use crate::object::{GcObject, GcRef};
use std::collections::HashMap;

pub struct Heap {
    objects: Vec<Option<GcObject>>,
    free_list: Vec<usize>,
    bytes_allocated: usize,
    next_gc: usize,
    intern_table: HashMap<u64, GcRef>, // string interning
}

impl Heap {
    pub const INITIAL_GC_THRESHOLD: usize = 1024 * 1024; // 1MB
    const GC_GROWTH_FACTOR: usize = 2;

    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            free_list: Vec::new(),
            bytes_allocated: 0,
            next_gc: Self::INITIAL_GC_THRESHOLD,
            intern_table: HashMap::new(),
        }
    }

    pub fn estimate_string_size(len: usize) -> usize {
        std::mem::size_of::<crate::object::AelysString>() + len
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

// clone gives you a fresh heap, not a copy (objects aren't clonable)
impl Clone for Heap {
    fn clone(&self) -> Self {
        Self::new()
    }
}
