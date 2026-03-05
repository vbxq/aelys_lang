use super::TypeEnv;
use crate::types::InferType;
use aelys_syntax::Span;

impl TypeEnv {
    /// Enter a new scope
    pub fn push_scope(&mut self) {
        self.locals.push(std::collections::HashMap::new());
        self.mutable_locals.push(std::collections::HashSet::new());
        self.function_scopes.push(std::collections::HashMap::new());
        self.binding_spans.push(std::collections::HashMap::new());
    }

    /// Exit the current scope
    pub fn pop_scope(&mut self) {
        if self.locals.len() > 1 {
            self.locals.pop();
            self.mutable_locals.pop();
            self.function_scopes.pop();
            self.binding_spans.pop();
        }
    }

    /// Define a local variable in the current scope
    pub fn define_local(&mut self, name: String, ty: InferType) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Define a local variable and record the span where it was first bound
    pub fn define_local_with_span(&mut self, name: String, ty: InferType, span: Span) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name.clone(), ty);
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

    /// Look up a variable (searches from innermost to outermost scope)
    pub fn lookup(&self, name: &str) -> Option<&InferType> {
        for scope in self.locals.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }

        if let Some(ty) = self.captures.get(name) {
            return Some(ty);
        }

        if let Some(ty) = self.lookup_function_ref(name) {
            return Some(ty);
        }

        None
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
        if let Some(scope) = self.mutable_locals.last_mut() {
            scope.insert(name);
        }
    }

    /// Check if a variable is mutable
    pub fn is_mutable(&self, name: &str) -> bool {
        // resolve mutability against the same lexical binding that lookup() would resolve
        // this prevents an inner `let mut x` from making an outer immutable `x` mutable
        for (scope, mutable_scope) in self.locals.iter().zip(self.mutable_locals.iter()).rev() {
            if scope.contains_key(name) {
                return mutable_scope.contains(name);
            }
        }

        if self.captures.contains_key(name) {
            return self.mutable_captures.contains(name);
        }

        false
    }
}
