use super::TypeEnv;
use crate::types::InferType;
use std::rc::Rc;

impl TypeEnv {
    /// Define a function signature (takes Rc to avoid cloning)
    pub fn define_function(&mut self, name: String, ty: Rc<InferType>) {
        if let Some(scope) = self.function_scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Define a function signature from an InferType (wraps in Rc)
    pub fn define_function_owned(&mut self, name: String, ty: InferType) {
        if let Some(scope) = self.function_scopes.last_mut() {
            scope.insert(name, Rc::new(ty));
        }
    }

    /// Check if a function is defined
    pub fn has_function(&self, name: &str) -> bool {
        self.function_scopes
            .last()
            .is_some_and(|scope| scope.contains_key(name))
    }

    /// Look up a function type (returns Rc for cheap cloning)
    pub fn lookup_function(&self, name: &str) -> Option<&Rc<InferType>> {
        for scope in self.function_scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    /// Look up a function type and get a reference to the inner type
    pub fn lookup_function_ref(&self, name: &str) -> Option<&InferType> {
        self.lookup_function(name).map(|rc| rc.as_ref())
    }

    /// Set the current function name (for recursive call resolution)
    pub fn set_current_function(&mut self, name: Option<String>) {
        self.current_function = name;
    }

    /// Get the current function name
    pub fn current_function(&self) -> Option<&String> {
        self.current_function.as_ref()
    }
}
