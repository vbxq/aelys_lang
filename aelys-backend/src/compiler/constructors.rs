use super::state::{Compiler, Local, Upvalue};
use aelys_bytecode::{Function, Heap};
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

impl Compiler {
    pub fn new(name: Option<String>, source: Arc<Source>) -> Self {
        Self {
            current: Function::new(name, 0),
            source,
            scopes: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            enclosing_locals: None,
            enclosing_upvalues: None,
            all_enclosing_locals: Vec::new(),
            loop_stack: Vec::new(),
            loop_variables: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            has_no_gc: false,
            heap: Heap::new(),
            register_pool: [false; 256],
            globals: HashMap::new(),
            global_indices: HashMap::new(),
            next_global_index: 0,
            module_aliases: Rc::new(HashSet::new()),
            known_globals: Rc::new(HashSet::new()),
            known_native_globals: Rc::new(HashSet::new()),
            symbol_origins: Rc::new(HashMap::new()),
            accessed_globals: HashSet::new(),
            next_call_site_slot: 0,
        }
    }

    // for REPL or nested fns
    pub fn with_heap_and_globals(
        name: Option<String>,
        source: Arc<Source>,
        heap: Heap,
        globals: HashMap<String, bool>,
    ) -> Self {
        Self {
            current: Function::new(name, 0),
            source,
            scopes: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            enclosing_locals: None,
            enclosing_upvalues: None,
            all_enclosing_locals: Vec::new(),
            loop_stack: Vec::new(),
            loop_variables: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            has_no_gc: false,
            heap,
            register_pool: [false; 256],
            globals,
            global_indices: HashMap::new(),
            next_global_index: 0,
            module_aliases: Rc::new(HashSet::new()),
            known_globals: Rc::new(HashSet::new()),
            known_native_globals: Rc::new(HashSet::new()),
            symbol_origins: Rc::new(HashMap::new()),
            accessed_globals: HashSet::new(),
            next_call_site_slot: 0,
        }
    }

    pub fn for_nested_function(
        name: Option<String>,
        source: Arc<Source>,
        heap: Heap,
        globals: HashMap<String, bool>,
        global_indices: HashMap<String, u16>,
        next_global_index: u16,
        enclosing_locals: Vec<Local>,
        enclosing_upvalues: Vec<Upvalue>,
        parent_all_enclosing_locals: Vec<Vec<Local>>,
        module_aliases: Rc<HashSet<String>>,
        known_globals: Rc<HashSet<String>>,
        known_native_globals: Rc<HashSet<String>>,
        symbol_origins: Rc<HashMap<String, String>>,
        next_call_site_slot: u16,
    ) -> Self {
        let mut all_enclosing_locals = vec![enclosing_locals.clone()];
        all_enclosing_locals.extend(parent_all_enclosing_locals);

        Self {
            current: Function::new(name, 0),
            source,
            scopes: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            enclosing_locals: Some(enclosing_locals),
            enclosing_upvalues: Some(enclosing_upvalues),
            all_enclosing_locals,
            loop_stack: Vec::new(),
            loop_variables: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            has_no_gc: false,
            heap,
            register_pool: [false; 256],
            globals,
            global_indices,
            next_global_index,
            module_aliases,
            known_globals,
            known_native_globals,
            symbol_origins,
            accessed_globals: HashSet::new(),
            next_call_site_slot,
        }
    }

    pub fn with_modules(
        name: Option<String>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
        known_native_globals: HashSet<String>,
        symbol_origins: HashMap<String, String>,
    ) -> Self {
        Self {
            current: Function::new(name, 0),
            source,
            scopes: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            enclosing_locals: None,
            enclosing_upvalues: None,
            all_enclosing_locals: Vec::new(),
            loop_stack: Vec::new(),
            loop_variables: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            has_no_gc: false,
            heap: Heap::new(),
            register_pool: [false; 256],
            globals: HashMap::new(),
            global_indices: HashMap::new(),
            next_global_index: 0,
            module_aliases: Rc::new(module_aliases),
            known_globals: Rc::new(known_globals),
            known_native_globals: Rc::new(known_native_globals),
            symbol_origins: Rc::new(symbol_origins),
            accessed_globals: HashSet::new(),
            next_call_site_slot: 0,
        }
    }

    // REPL + modules
    pub fn with_modules_and_globals(
        name: Option<String>,
        source: Arc<Source>,
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
        known_native_globals: HashSet<String>,
        symbol_origins: HashMap<String, String>,
        globals: HashMap<String, bool>,
    ) -> Self {
        Self {
            current: Function::new(name, 0),
            source,
            scopes: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
            enclosing_locals: None,
            enclosing_upvalues: None,
            all_enclosing_locals: Vec::new(),
            loop_stack: Vec::new(),
            loop_variables: Vec::new(),
            scope_depth: 0,
            next_register: 0,
            has_no_gc: false,
            heap: Heap::new(),
            register_pool: [false; 256],
            globals,
            global_indices: HashMap::new(),
            next_global_index: 0,
            module_aliases: Rc::new(module_aliases),
            known_globals: Rc::new(known_globals),
            known_native_globals: Rc::new(known_native_globals),
            symbol_origins: Rc::new(symbol_origins),
            accessed_globals: HashSet::new(),
            next_call_site_slot: 0,
        }
    }
}
