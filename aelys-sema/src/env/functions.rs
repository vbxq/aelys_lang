use super::TypeEnv;
use crate::types::InferType;
use std::rc::Rc;

impl TypeEnv {
    /// Define a function signature (takes Rc to avoid cloning)
    pub fn define_function(&mut self, name: String, ty: Rc<InferType>) {
        self.functions.insert(name, ty);
    }

    /// Define a function signature from an InferType (wraps in Rc)
    pub fn define_function_owned(&mut self, name: String, ty: InferType) {
        self.functions.insert(name, Rc::new(ty));
    }

    /// Look up a function type (returns Rc for cheap cloning)
    pub fn lookup_function(&self, name: &str) -> Option<&Rc<InferType>> {
        self.functions.get(name)
    }

    /// Look up a function type and get a reference to the inner type
    pub fn lookup_function_ref(&self, name: &str) -> Option<&InferType> {
        self.functions.get(name).map(|rc| rc.as_ref())
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
