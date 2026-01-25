use super::super::{Compiler, Local};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

impl Compiler {
    pub fn declare_variable(&mut self, name: &str, mutable: bool) -> Result<u8> {
        for local in self.locals.iter().rev() {
            if local.depth < self.scope_depth {
                break;
            }
            if local.name == name {
                return Err(CompileError::new(
                    CompileErrorKind::VariableAlreadyDefined(name.to_string()),
                    Span::dummy(),
                    self.source.clone(),
                )
                .into());
            }
        }

        let register = self.alloc_register()?;

        self.locals.push(Local {
            name: name.to_string(),
            depth: self.scope_depth,
            mutable,
            register,
            is_captured: false,
            resolved_type: aelys_sema::ResolvedType::Dynamic,
            is_freed: false,
        });

        Ok(register)
    }

    pub fn mark_local_captured(&mut self, register: u8) {
        for local in self.locals.iter_mut() {
            if local.register == register {
                local.is_captured = true;
                break;
            }
        }

        if let Some(scope) = self.scopes.last_mut() {
            if !scope.captured_registers.contains(&register) {
                scope.captured_registers.push(register);
            }
        }
    }
}
