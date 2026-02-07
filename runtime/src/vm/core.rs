use super::config::VmConfig;
use super::frame::CallFrame;
use super::manual_heap::ManualHeap;
use super::{GcRef, Heap, NativeFunctionImpl, Value};
use crate::native::NativeModule;
use crate::stdlib::Resource;
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const MAX_FRAMES: usize = 1024;
pub const MAX_REGISTERS: usize = 65536;
pub const MAX_NO_GC_DEPTH: usize = 64;
pub const MAX_CALL_SITE_SLOTS: usize = 4096;

// windowed regs like Lua
pub struct VM {
    pub(crate) heap: Heap,
    pub(crate) config: VmConfig,
    pub(crate) manual_heap: ManualHeap,
    pub(crate) registers: Vec<Value>,
    pub(crate) frames: Vec<CallFrame>,
    pub(crate) globals: HashMap<String, Value>,
    pub(crate) global_mutability: HashMap<String, bool>,
    pub(crate) globals_by_index_cache: HashMap<usize, Arc<Vec<Value>>>,
    pub(crate) globals_by_index: Vec<Value>,
    pub(crate) source: Arc<Source>,
    pub(crate) no_gc_depth: usize,
    pub(crate) open_upvalues: Vec<GcRef>,
    pub(crate) current_upvalues: Vec<GcRef>,
    pub(crate) call_site_cache: Vec<CallSiteCacheEntry>,
    pub(crate) resources: Vec<Option<Resource>>,
    pub(crate) native_modules: HashMap<String, NativeModule>,
    pub(crate) native_registry: HashMap<String, NativeFunctionImpl>,

    pub(crate) current_global_mapping_id: usize,
    pub(crate) program_args: Vec<String>,
    pub(crate) script_path: Option<String>,
    pub(crate) repl_module_aliases: HashSet<String>,
    pub(crate) repl_known_globals: HashSet<String>,
    pub(crate) repl_known_native_globals: HashSet<String>,
    pub(crate) repl_symbol_origins: HashMap<String, String>,
}

// MIC entry for CallGlobal - avoids repeat lookups
#[derive(Clone, Copy)]
#[repr(C, align(8))]
pub struct CallSiteCacheEntry {
    pub bytecode_ptr: *const u32,
    pub constants_ptr: *const Value,
    pub bytecode_len: u32,
    pub constants_len: u16,
    pub arity: u8,
    pub num_registers: u8,
    pub callee_gmap: usize,
    pub is_closure: bool,
}

impl Default for CallSiteCacheEntry {
    fn default() -> Self {
        Self {
            bytecode_ptr: std::ptr::null(),
            constants_ptr: std::ptr::null(),
            bytecode_len: 0,
            constants_len: 0,
            arity: 0,
            num_registers: 0,
            callee_gmap: 0,
            is_closure: false,
        }
    }
}

unsafe impl Send for CallSiteCacheEntry {}
unsafe impl Sync for CallSiteCacheEntry {}

#[derive(Debug)]
pub enum StepResult {
    Continue,
    Return(Value),
}
