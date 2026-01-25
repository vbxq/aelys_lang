use super::TypeEnv;
use std::collections::HashMap;

impl TypeEnv {
    /// Clone with fresh captures (for entering a new function)
    pub fn for_function(&self) -> TypeEnv {
        TypeEnv {
            locals: vec![HashMap::new()],
            captures: HashMap::new(),
            functions: self.functions.clone(),
            current_function: None,
        }
    }

    /// Clone with inherited captures (for closures)
    pub fn for_closure(&self) -> TypeEnv {
        let mut all_visible = HashMap::new();

        for scope in &self.locals {
            for (name, ty) in scope {
                all_visible.insert(name.clone(), ty.clone());
            }
        }

        for (name, ty) in &self.captures {
            all_visible.insert(name.clone(), ty.clone());
        }

        TypeEnv {
            locals: vec![HashMap::new()],
            captures: all_visible,
            functions: self.functions.clone(),
            current_function: None,
        }
    }
}
