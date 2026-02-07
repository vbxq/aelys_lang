use aelys_sema::TypedExpr;
use std::collections::HashMap;

pub struct ScopeStack {
    scopes: Vec<HashMap<String, TypedExpr>>,
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
            scope.insert(name, expr);
        }
    }

    pub fn get(&self, name: &str) -> Option<&TypedExpr> {
        for scope in self.scopes.iter().rev() {
            if let Some(expr) = scope.get(name) {
                return Some(expr);
            }
        }
        None
    }

    pub fn invalidate(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.remove(name).is_some() {
                return;
            }
        }
    }
}
