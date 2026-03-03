//! Type environment for inference.

mod captures;
mod closure;
mod free_vars;
mod functions;
mod scope;

use crate::types::InferType;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

/// Type environment - maps names to types
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    /// Local variables in current scope (name -> type)
    locals: Vec<HashMap<String, InferType>>,

    /// Captured variables from enclosing scopes (upvalues)
    captures: HashMap<String, InferType>,

    /// Known function signatures (name -> function type)
    /// Uses Rc to avoid cloning function types during lookup
    functions: HashMap<String, Rc<InferType>>,

    /// Current function name (for recursive calls)
    current_function: Option<String>,

    /// Mutable local bindings per lexical scope (parallels `locals`)
    mutable_locals: Vec<HashSet<String>>,

    /// Mutable names inherited as captures in closure environments
    mutable_captures: HashSet<String>,
}

impl TypeEnv {
    pub fn new() -> Self {
        Self {
            locals: vec![HashMap::new()],
            captures: HashMap::new(),
            functions: HashMap::new(),
            current_function: None,
            mutable_locals: vec![HashSet::new()],
            mutable_captures: HashSet::new(),
        }
    }
}
