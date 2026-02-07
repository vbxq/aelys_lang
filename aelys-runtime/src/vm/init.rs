use super::config::{VMCapabilities, VmConfig};
use super::manual_heap::ManualHeap;
use super::{Heap, Value};
use super::{MAX_FRAMES, MAX_REGISTERS, VM};
use aelys_common::error::RuntimeError;
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

impl VM {
    pub fn new(source: Arc<Source>) -> Result<Self, RuntimeError> {
        Self::with_config_and_args(source, VmConfig::default(), Vec::new())
    }

    pub fn with_capabilities(
        source: Arc<Source>,
        capabilities: VMCapabilities,
    ) -> Result<Self, RuntimeError> {
        let config = VmConfig {
            capabilities,
            ..VmConfig::default()
        };
        Self::with_config_and_args(source, config, Vec::new())
    }

    pub fn with_config(source: Arc<Source>, config: VmConfig) -> Result<Self, RuntimeError> {
        Self::with_config_and_args(source, config, Vec::new())
    }

    pub fn with_config_and_args(
        source: Arc<Source>,
        config: VmConfig,
        program_args: Vec<String>,
    ) -> Result<Self, RuntimeError> {
        let mut vm = Self {
            heap: Heap::new(),
            config,
            manual_heap: ManualHeap::new(),
            registers: {
                let mut regs = Vec::with_capacity(MAX_REGISTERS);
                regs.resize(32768, Value::null());
                regs
            },
            frames: Vec::with_capacity(MAX_FRAMES),
            globals: HashMap::new(),
            global_mutability: HashMap::new(),
            globals_by_index_cache: HashMap::with_capacity(32),
            globals_by_index: Vec::with_capacity(64),
            source,
            no_gc_depth: 0,
            open_upvalues: Vec::new(),
            current_upvalues: Vec::new(),
            call_site_cache: Vec::with_capacity(64),
            resources: Vec::with_capacity(16),
            native_modules: HashMap::new(),
            native_registry: HashMap::new(),
            current_global_mapping_id: 0,
            program_args,
            script_path: None,
            repl_module_aliases: HashSet::new(),
            repl_known_globals: HashSet::new(),
            repl_known_native_globals: HashSet::new(),
            repl_symbol_origins: HashMap::new(),
        };
        super::builtins::register_builtins(&mut vm)?;
        Ok(vm)
    }
}
