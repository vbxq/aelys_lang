use super::super::Compiler;
use aelys_common::Result;
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_member_access(
        &mut self,
        object: &aelys_sema::TypedExpr,
        member: &str,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        use aelys_sema::TypedExprKind;

        if let TypedExprKind::Identifier(module_name) = &object.kind {
            if self.module_aliases.contains(module_name) {
                let global_name = format!("{}::{}", module_name, member);
                let idx = self.get_or_create_global_index(&global_name);
                self.accessed_globals.insert(global_name.clone());
                self.emit_b(aelys_bytecode::OpCode::GetGlobalIdx, dest, idx as i16, span);
                return Ok(());
            }
        }

        self.compile_identifier(member, dest, span)
    }
}
