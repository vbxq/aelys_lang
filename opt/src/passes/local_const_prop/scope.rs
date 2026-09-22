use aelys_sema::TypedExpr;
use std::collections::HashMap;

enum ScopeEntry {
    Value(TypedExpr),
    Blocked,
}

pub struct ScopeStack {
    scopes: Vec<HashMap<String, ScopeEntry>>,
}

impl ScopeStack {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub fn insert(&mut self, name: String, expr: TypedExpr) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ScopeEntry::Value(expr));
        }
    }

    /// Shadow `name` in the current scope so outer constant bindings are invisible.
    /// Used when a match arm pattern introduces a binding that shadows an outer let.
    pub fn block(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ScopeEntry::Blocked);
        }
    }

    pub fn get(&self, name: &str) -> Option<&TypedExpr> {
        for scope in self.scopes.iter().rev() {
            match scope.get(name) {
                Some(ScopeEntry::Value(expr)) => return Some(expr),
                Some(ScopeEntry::Blocked) => return None,
                None => continue,
            }
        }
        None
    }

    pub fn invalidate(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), ScopeEntry::Blocked);
                return;
            }
        }
    }
}
