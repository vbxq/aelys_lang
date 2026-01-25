use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::ast::{BinaryOp, Expr, ExprKind};

impl Compiler {
    pub fn compile_expression_stmt(&mut self, expr: &Expr) -> Result<()> {
        match &expr.kind {
            ExprKind::Assign { name, value } => {
                if let Some((reg, mutable)) = self.resolve_variable(name) {
                    if !mutable {
                        return Err(CompileError::new(
                            CompileErrorKind::AssignToImmutable(name.to_string()),
                            expr.span,
                            self.source.clone(),
                        )
                        .into());
                    }
                    if self.loop_variables.contains(name) {
                        return Err(CompileError::new(
                            CompileErrorKind::AssignToLoopVariable(name.to_string()),
                            expr.span,
                            self.source.clone(),
                        )
                        .into());
                    }
                    if let ExprKind::Binary {
                        left,
                        op: BinaryOp::Add,
                        right,
                    } = &value.kind
                    {
                        if let (ExprKind::Identifier(id), ExprKind::Int(n)) =
                            (&left.kind, &right.kind)
                        {
                            if id == name && *n >= 0 && *n <= 255 {
                                self.emit_a(OpCode::AddI, reg, reg, *n as u8, expr.span);
                                return Ok(());
                            }
                        }
                        if let (ExprKind::Int(n), ExprKind::Identifier(id)) =
                            (&left.kind, &right.kind)
                        {
                            if id == name && *n >= 0 && *n <= 255 {
                                self.emit_a(OpCode::AddI, reg, reg, *n as u8, expr.span);
                                return Ok(());
                            }
                        }
                    }
                    if let ExprKind::Binary {
                        left,
                        op: BinaryOp::Sub,
                        right,
                    } = &value.kind
                    {
                        if let (ExprKind::Identifier(id), ExprKind::Int(n)) =
                            (&left.kind, &right.kind)
                        {
                            if id == name && *n >= 0 && *n <= 255 {
                                self.emit_a(OpCode::SubI, reg, reg, *n as u8, expr.span);
                                return Ok(());
                            }
                        }
                    }
                    self.compile_expr(value, reg)?;
                    return Ok(());
                }
            }
            _ => {}
        }
        let temp = self.alloc_register()?;
        self.compile_expr(expr, temp)?;
        self.free_register(temp);
        Ok(())
    }
}
