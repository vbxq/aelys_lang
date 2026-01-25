use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, TypeAnnotation};

impl Compiler {
    pub fn compile_let(
        &mut self,
        name: &str,
        mutable: bool,
        _type_annotation: Option<&TypeAnnotation>,
        initializer: &Expr,
        _is_pub: bool,
    ) -> Result<()> {
        if self.scope_depth == 0 {
            self.globals.insert(name.to_string(), mutable);

            let global_idx = if let Some(&idx) = self.global_indices.get(name) {
                idx
            } else {
                let idx = self.next_global_index;
                self.global_indices.insert(name.to_string(), idx);
                self.next_global_index += 1;
                idx
            };

            let temp_reg = self.alloc_register()?;
            self.compile_expr(initializer, temp_reg)?;

            self.accessed_globals.insert(name.to_string());
            self.emit_b(
                OpCode::SetGlobalIdx,
                temp_reg,
                global_idx as i16,
                Span::dummy(),
            );
            self.free_register(temp_reg);
        } else {
            let reg = self.declare_variable(name, mutable)?;
            self.compile_expr(initializer, reg)?;
        }
        Ok(())
    }

    pub fn compile_typed_let(
        &mut self,
        name: &str,
        mutable: bool,
        initializer: &aelys_sema::TypedExpr,
        var_type: &aelys_sema::ResolvedType,
        _is_pub: bool,
        span: Span,
    ) -> Result<()> {
        if self.scope_depth == 0 {
            let reg = self.alloc_register()?;
            self.compile_typed_expr(initializer, reg)?;

            self.globals.insert(name.to_string(), mutable);
            let idx = self.get_or_create_global_index(name);
            self.accessed_globals.insert(name.to_string());

            self.emit_b(OpCode::SetGlobalIdx, reg, idx as i16, span);
            self.free_register(reg);
        } else {
            let reg = self.alloc_register()?;
            self.compile_typed_expr(initializer, reg)?;

            self.add_local(name.to_string(), mutable, reg, var_type.clone());
        }

        Ok(())
    }
}
