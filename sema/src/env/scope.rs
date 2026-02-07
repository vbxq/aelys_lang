use super::TypeEnv;
use crate::types::InferType;

impl TypeEnv {
    /// Enter a new scope
    pub fn push_scope(&mut self) {
        self.locals.push(std::collections::HashMap::new());
    }

    /// Exit the current scope
    pub fn pop_scope(&mut self) {
        if self.locals.len() > 1 {
            self.locals.pop();
        }
    }

    /// Define a local variable in the current scope
    pub fn define_local(&mut self, name: String, ty: InferType) {
        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name, ty);
        }
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

        if let Some(ty) = self.functions.get(name) {
            return Some(ty.as_ref());
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
}
