use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_assign(
        &mut self,
        name: &str,
        value: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_expr(value, dest)?;

        if let Some((reg, mutable)) = self.resolve_variable(name) {
            if !mutable {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }
            if reg != dest {
                self.emit_a(OpCode::Move, reg, dest, 0, span);
            }
        } else if let Some((upval_idx, mutable)) = self.resolve_upvalue(name) {
            if !mutable {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }
            self.emit_a(OpCode::SetUpval, upval_idx, dest, 0, span);
        } else {
            if let Some(&mutable) = self.globals.get(name)
                && !mutable
            {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }
            // For assignments to user-defined globals, use raw index without translation
            let idx = self.get_or_create_global_index_raw(name);
            self.accessed_globals.insert(name.to_string());
            self.emit_b(OpCode::SetGlobalIdx, dest, idx as i16, span);
        }

        Ok(())
    }
}
