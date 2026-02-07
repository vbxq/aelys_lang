use super::{Compiler, Local};

impl Compiler {
    pub fn add_local(
        &mut self,
        name: String,
        mutable: bool,
        register: u8,
        resolved_type: aelys_sema::ResolvedType,
    ) {
        self.locals.push(Local {
            name,
            depth: self.scope_depth,
            mutable,
            register,
            is_captured: false,
            resolved_type,
            is_freed: false,
        });
    }
}
