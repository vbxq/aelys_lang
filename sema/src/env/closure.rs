use super::TypeEnv;
use std::collections::HashMap;

impl TypeEnv {
    /// Clone with inherited captures (for named functions and lambdas).
    ///
    /// In Aelys, named functions use closure semantics: parent-scope captures
    /// are allowed. That's an intentional design decision (similar to Go/JS).
    pub fn for_closure(&self) -> TypeEnv {
        let mut all_visible = HashMap::new();

        // Insert captures first, then locals overwrite in case of collision
        // This ensures locals have priority over captures
        for (name, ty) in &self.captures {
            all_visible.insert(name.clone(), ty.clone());
        }

        for scope in &self.locals {
            for (name, ty) in scope {
                all_visible.insert(name.clone(), ty.clone());
            }
        }

        TypeEnv {
            locals: vec![HashMap::new()],
            captures: all_visible,
            functions: self.functions.clone(),
            current_function: None,
            mutable_vars: self.mutable_vars.clone(),
        }
    }
}
