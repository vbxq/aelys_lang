use super::TypeEnv;
use crate::types::InferType;
use aelys_syntax::Span;
use std::rc::Rc;

impl TypeEnv {
    /// Enter a new scope
    pub fn push_scope(&mut self) {
        self.locals.push(Rc::default());
        self.function_scopes.push(Rc::default());
        self.binding_spans.push(std::collections::HashMap::new());
    }

    /// Exit the current scope
    pub fn pop_scope(&mut self) {
        if self.locals.len() > 1 {
            self.locals.pop();
            self.function_scopes.pop();
            self.binding_spans.pop();
        }
    }

    /// Define a local variable in the current scope
    pub fn define_local(&mut self, name: String, ty: InferType) {
        if let Some(scope) = self.locals.last_mut() {
            let scope = Rc::make_mut(scope);
            scope.ids.insert(name.clone(), super::BindingId::fresh());
            scope.types.insert(name, ty);
        }
    }

    /// Define a local variable and record the span where it was first bound
    pub fn define_local_with_span(&mut self, name: String, ty: InferType, span: Span) {
        if let Some(scope) = self.locals.last_mut() {
            let scope = Rc::make_mut(scope);
            scope.ids.insert(name.clone(), super::BindingId::fresh());
            scope.types.insert(name.clone(), ty);
        }
        if let Some(spans) = self.binding_spans.last_mut() {
            spans.insert(name, span);
        }
    }

    /// Look up the span where a variable was first bound
    pub fn lookup_binding_span(&self, name: &str) -> Option<Span> {
        for scope in self.binding_spans.iter().rev() {
            if let Some(span) = scope.get(name) {
                return Some(*span);
            }
        }
        None
    }

    pub fn lookup_binding_id(&self, name: &str) -> Option<super::BindingId> {
        self.binding_scope(name)
            .and_then(|scope| scope.ids.get(name).copied())
    }

    fn binding_scope(&self, name: &str) -> Option<&super::VarScope> {
        self.captures
            .iter()
            .chain(&self.locals)
            .rev()
            .map(|scope| scope.as_ref())
            .find(|scope| scope.types.contains_key(name))
    }

    /// Look up a variable (searches from innermost to outermost scope)
    pub fn lookup(&self, name: &str) -> Option<&InferType> {
        self.lookup_local(name)
            .or_else(|| self.lookup_function_ref(name))
    }

    /// look up a variable among locals and captures only, never function signatures.
    /// mirrors the bir's name_map (locals-only) so a fn name is not resolved as a local.
    pub fn lookup_local(&self, name: &str) -> Option<&InferType> {
        self.binding_scope(name)
            .and_then(|scope| scope.types.get(name))
    }

    /// Check if a variable exists
    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Current scope depth
    pub fn depth(&self) -> usize {
        self.locals.len()
    }

    /// Mark a variable as mutable
    pub fn mark_mutable(&mut self, name: String) {
        if let Some(scope) = self.locals.last_mut() {
            Rc::make_mut(scope).mutable.insert(name);
        }
    }

    /// Check if a variable is mutable
    pub fn is_mutable(&self, name: &str) -> bool {
        // resolve mutability against the same lexical binding that lookup() would resolve
        // this prevents an inner `let mut x` from making an outer immutable `x` mutable
        self.binding_scope(name)
            .is_some_and(|scope| scope.mutable.contains(name))
    }
}
