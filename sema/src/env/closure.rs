use super::TypeEnv;
use std::collections::HashMap;
use std::rc::Rc;

impl TypeEnv {
    /// Clone with inherited captures (for named functions and lambdas).
    ///
    /// In Aelys, named functions use closure semantics: parent-scope captures
    /// are allowed. That's an intentional design decision (similar to Go/JS).
    pub fn for_closure(&self) -> TypeEnv {
        TypeEnv {
            locals: vec![Rc::default()],
            captures: self.captures.iter().chain(&self.locals).cloned().collect(),
            function_scopes: self
                .function_scopes
                .iter()
                .cloned()
                .chain([Rc::default()])
                .collect(),
            own_function_scope: self.function_scopes.len(),
            current_function: None,
            binding_spans: vec![HashMap::new()],
        }
    }
}
