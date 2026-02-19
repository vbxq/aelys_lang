use aelys_bytecode::{Function, Heap};
use aelys_sema::ResolvedType;
use aelys_syntax::Source;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Local {
    pub name: String,
    pub depth: usize,
    pub mutable: bool,
    pub register: u8,
    pub is_captured: bool, // closure capture
    pub resolved_type: ResolvedType,
    pub is_freed: bool, // liveness freed this reg
}

#[derive(Debug, Clone)]
pub struct Upvalue {
    pub is_local: bool, // true = from enclosing locals, false = from enclosing upvalues
    pub index: u8,      // reg if is_local, upvalue idx otherwise
    pub name: String,
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub struct Scope {
    pub start: usize,
    pub captured_registers: Vec<u8>, // for CloseUpvals
}

#[derive(Debug, Clone)]
pub struct LoopContext {
    pub start: usize,
    pub break_jumps: Vec<usize>,
    pub continue_jumps: Vec<usize>, // for-loop: forward patch to increment
    pub is_for_loop: bool,          // for: continue forward, while: continue back
}

pub struct Compiler {
    pub current: Function,
    pub source: Arc<Source>,
    pub scopes: Vec<Scope>,
    pub locals: Vec<Local>,
    pub upvalues: Vec<Upvalue>,
    pub enclosing_locals: Option<Vec<Local>>,
    pub enclosing_upvalues: Option<Vec<Upvalue>>,
    pub all_enclosing_locals: Vec<Vec<Local>>, // transitive capture chain
    pub loop_stack: Vec<LoopContext>,
    pub loop_variables: Vec<String>, // can't assign inside loop body
    pub scope_depth: usize,
    pub next_register: u8,
    pub has_no_gc: bool,
    pub heap: Heap,
    pub(crate) register_pool: [bool; 256],
    pub globals: HashMap<String, bool>, // name -> mutable
    pub global_indices: HashMap<String, u16>,
    pub next_global_index: u16,
    pub module_aliases: Rc<HashSet<String>>,
    pub known_globals: Rc<HashSet<String>>,
    pub known_native_globals: Rc<HashSet<String>>,
    pub symbol_origins: Rc<HashMap<String, String>>, // bare name -> qualified name
    pub accessed_globals: HashSet<String>,
    pub next_call_site_slot: u16,
    pub function_depth: usize,
}
