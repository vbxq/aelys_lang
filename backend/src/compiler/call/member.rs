use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    pub fn compile_member_access(
        &mut self,
        object: &Expr,
        member: &str,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        if let ExprKind::Identifier(module_name) = &object.kind
            && self.module_aliases.contains(module_name)
        {
            let qualified_name = format!("{}::{}", module_name, member);

            let idx = if let Some(&idx) = self.global_indices.get(&qualified_name) {
                idx
            } else {
                let idx = self.next_global_index;
                self.global_indices.insert(qualified_name.clone(), idx);
                self.next_global_index += 1;
                idx
            };

            self.accessed_globals.insert(qualified_name);
            self.emit_b(OpCode::GetGlobalIdx, dest, idx as i16, span);
            return Ok(());
        }

        if Self::is_builtin(member) {
            let idx = if let Some(&idx) = self.global_indices.get(member) {
                idx
            } else {
                let idx = self.next_global_index;
                self.global_indices.insert(member.to_string(), idx);
                self.next_global_index += 1;
                idx
            };
            self.accessed_globals.insert(member.to_string());
            self.emit_b(OpCode::GetGlobalIdx, dest, idx as i16, span);
            Ok(())
        } else {
            Err(CompileError::new(
                CompileErrorKind::UndefinedVariable(member.to_string()),
                span,
                self.source.clone(),
            )
            .into())
        }
    }
}
