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

#[derive(Debug, Clone, Default)]
struct VarScope {
    types: HashMap<String, InferType>,
    mutable: HashSet<String>,
    ids: HashMap<String, BindingId>,
}

/// Type environment - maps names to types
#[derive(Debug, Clone, Default)]
pub struct TypeEnv {
    /// Local variables in current scope (name -> type)
    locals: Vec<Rc<VarScope>>,

    // the enclosing functions' scopes, shared rather than copied, innermost last
    captures: Vec<Rc<VarScope>>,

    /// Function signatures by lexical scope (name -> function type)
    /// Uses Rc to avoid cloning function types during lookup
    function_scopes: Vec<Rc<HashMap<String, Rc<InferType>>>>,

    // index of the function's own scope; the scopes below it are inherited from enclosing functions
    own_function_scope: usize,

    /// Current function name (for recursive calls)
    current_function: Option<String>,

    /// Binding spans per lexical scope (parallels `locals`): where each variable was first defined
    binding_spans: Vec<HashMap<String, Span>>,
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
            locals: vec![Rc::default()],
            captures: Vec::new(),
            function_scopes: vec![Rc::default()],
            own_function_scope: 0,
            current_function: None,
            binding_spans: vec![HashMap::new()],
        }
    }
}
