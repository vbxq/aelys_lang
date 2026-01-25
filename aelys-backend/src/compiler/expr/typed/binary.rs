use super::super::Compiler;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::BinaryOp;

impl Compiler {
    fn get_typed_local_register(&self, expr: &aelys_sema::TypedExpr) -> Option<u8> {
        if let aelys_sema::TypedExprKind::Identifier(name) = &expr.kind {
            if let Some((reg, _)) = self.resolve_variable(name) {
                return Some(reg);
            }
        }
        None
    }

    pub(super) fn compile_typed_binary(
        &mut self,
        left: &aelys_sema::TypedExpr,
        op: BinaryOp,
        right: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let left_resolved = aelys_sema::ResolvedType::from_infer_type(&left.ty);
        let right_resolved = aelys_sema::ResolvedType::from_infer_type(&right.ty);
        let opcode = crate::opcode_select::select_opcode(op, &left_resolved, &right_resolved);

        if let Some(left_local_reg) = self.get_typed_local_register(left) {
            let (right_reg, right_needs_free) =
                if let Some(r) = self.get_typed_local_register(right) {
                    (r, false)
                } else {
                    let temp = self.alloc_register()?;
                    self.compile_typed_expr(right, temp)?;
                    (temp, true)
                };

            self.emit_a(opcode, dest, left_local_reg, right_reg, span);

            if right_needs_free {
                self.free_register(right_reg);
            }
            return Ok(());
        }

        if let Some(right_local_reg) = self.get_typed_local_register(right) {
            self.compile_typed_expr(left, dest)?;
            self.emit_a(opcode, dest, dest, right_local_reg, span);
            return Ok(());
        }

        let right_has_side_effects = Self::typed_expr_may_have_side_effects(right);

        if right_has_side_effects {
            let left_reg = self.alloc_register()?;
            self.compile_typed_expr(left, left_reg)?;

            let right_reg = self.alloc_register()?;
            self.compile_typed_expr(right, right_reg)?;

            self.emit_a(opcode, dest, left_reg, right_reg, span);

            self.free_register(right_reg);
            self.free_register(left_reg);
        } else {
            self.compile_typed_expr(left, dest)?;

            let right_reg = self.alloc_register()?;
            self.compile_typed_expr(right, right_reg)?;

            self.emit_a(opcode, dest, dest, right_reg, span);

            self.free_register(right_reg);
        }

        Ok(())
    }
}
