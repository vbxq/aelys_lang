//! Type environment for inference.

mod captures;
mod closure;
mod free_vars;
mod functions;
mod scope;

use crate::types::InferType;
use aelys_syntax::Span;
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

    /// Function signatures by lexical scope (name -> function type)
    /// Uses Rc to avoid cloning function types during lookup
    function_scopes: Vec<HashMap<String, Rc<InferType>>>,

    /// Current function name (for recursive calls)
    current_function: Option<String>,

    /// Mutable local bindings per lexical scope (parallels `locals`)
    mutable_locals: Vec<HashSet<String>>,

    /// Mutable names inherited as captures in closure environments
    mutable_captures: HashSet<String>,

    /// Binding spans per lexical scope (parallels `locals`): where each variable was first defined
    binding_spans: Vec<HashMap<String, Span>>,

    binding_ids: Vec<HashMap<String, BindingId>>,
    capture_ids: HashMap<String, BindingId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingId(u64);

impl BindingId {
    pub(crate) fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        BindingId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

impl TypeEnv {
    pub fn new() -> Self {
        Self {
            locals: vec![HashMap::new()],
            captures: HashMap::new(),
            function_scopes: vec![HashMap::new()],
            current_function: None,
            mutable_locals: vec![HashSet::new()],
            mutable_captures: HashSet::new(),
            binding_spans: vec![HashMap::new()],
            binding_ids: vec![HashMap::new()],
            capture_ids: HashMap::new(),
        }
    }
}
