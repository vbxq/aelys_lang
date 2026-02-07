use super::super::{Compiler, Upvalue};

// TODO: this clone dance for enclosing_locals is ugly, maybe Rc
impl Compiler {
    pub fn resolve_variable_typed(
        &self,
        name: &str,
    ) -> Option<(u8, bool, &aelys_sema::ResolvedType)> {
        for local in self.locals.iter().rev() {
            if local.is_freed {
                continue;
            }
            if local.name == name {
                return Some((local.register, local.mutable, &local.resolved_type));
            }
        }
        None
    }

    pub fn resolve_variable(&self, name: &str) -> Option<(u8, bool)> {
        for local in self.locals.iter().rev() {
            if local.is_freed {
                continue;
            }
            if local.name == name {
                return Some((local.register, local.mutable));
            }
        }
        None
    }

    pub fn resolve_upvalue(&mut self, name: &str) -> Option<(u8, bool)> {
        for (i, upvalue) in self.upvalues.iter().enumerate() {
            if upvalue.name == name {
                return Some((i as u8, upvalue.mutable));
            }
        }

        if let Some(ref mut enclosing_locals) = self.enclosing_locals.clone() {
            for (i, local) in enclosing_locals.iter().enumerate() {
                if local.name == name {
                    if let Some(ref mut locals) = self.enclosing_locals {
                        locals[i].is_captured = true;
                    }

                    let upvalue_index = self.upvalues.len() as u8;
                    self.upvalues.push(Upvalue {
                        is_local: true,
                        index: local.register,
                        name: name.to_string(),
                        mutable: local.mutable,
                    });
                    return Some((upvalue_index, local.mutable));
                }
            }
        }

        if let Some(ref enclosing_upvalues) = self.enclosing_upvalues.clone() {
            for (i, upvalue) in enclosing_upvalues.iter().enumerate() {
                if upvalue.name == name {
                    let upvalue_index = self.upvalues.len() as u8;
                    self.upvalues.push(Upvalue {
                        is_local: false,
                        index: i as u8,
                        name: name.to_string(),
                        mutable: upvalue.mutable,
                    });
                    return Some((upvalue_index, upvalue.mutable));
                }
            }
        }

        for (depth, ancestor_locals) in self.all_enclosing_locals.iter().enumerate().skip(1) {
            for local in ancestor_locals.iter() {
                if local.name == name {
                    let upvalue_index = self.upvalues.len() as u8;
                    self.upvalues.push(Upvalue {
                        is_local: false,
                        index: (depth - 1) as u8 | 0x80,
                        name: name.to_string(),
                        mutable: local.mutable,
                    });
                    return Some((upvalue_index, local.mutable));
                }
            }
        }

        None
    }
}
