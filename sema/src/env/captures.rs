use super::TypeEnv;
use crate::types::InferType;
use std::rc::Rc;

impl TypeEnv {
    /// Define a captured variable (upvalue)
    pub fn define_capture(&mut self, name: String, ty: InferType) {
        if self.captures.is_empty() {
            self.captures.push(Rc::default());
        }
        let scope = Rc::make_mut(self.captures.last_mut().unwrap());
        scope.ids.insert(name.clone(), super::BindingId::fresh());
        scope.types.insert(name, ty);
    }

    pub fn lookup_capture(&self, name: &str) -> Option<&InferType> {
        self.captures
            .iter()
            .rev()
            .find_map(|scope| scope.types.get(name))
    }
}
