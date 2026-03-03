use super::TypeEnv;
use std::collections::{HashMap, HashSet};

impl TypeEnv {
    /// Clone with inherited captures (for named functions and lambdas).
    ///
    /// In Aelys, named functions use closure semantics: parent-scope captures
    /// are allowed. That's an intentional design decision (similar to Go/JS).
    pub fn for_closure(&self) -> TypeEnv {
        let mut all_visible = HashMap::new();
        let mut mutable_by_name = HashMap::new();
        let mut all_functions = HashMap::new();

        // Insert captures first, then locals overwrite in case of collision
        // This ensures locals have priority over captures
        for (name, ty) in &self.captures {
            all_visible.insert(name.clone(), ty.clone());
            mutable_by_name.insert(name.clone(), self.mutable_captures.contains(name));
        }

        for (scope, mutable_scope) in self.locals.iter().zip(self.mutable_locals.iter()) {
            for (name, ty) in scope {
                all_visible.insert(name.clone(), ty.clone());
                mutable_by_name.insert(name.clone(), mutable_scope.contains(name));
            }
        }

        for scope in &self.function_scopes {
            for (name, ty) in scope {
                all_functions.insert(name.clone(), ty.clone());
            }
        }

        let all_mutable = mutable_by_name
            .into_iter()
            .filter_map(|(name, is_mut)| is_mut.then_some(name))
            .collect();

        TypeEnv {
            locals: vec![HashMap::new()],
            captures: all_visible,
            function_scopes: vec![all_functions],
            current_function: None,
            mutable_locals: vec![HashSet::new()],
            mutable_captures: all_mutable,
        }
    }
}
