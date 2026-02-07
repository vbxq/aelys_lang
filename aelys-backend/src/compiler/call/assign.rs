use super::super::Compiler;
use aelys_bytecode::{OpCode, Value};
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{BinaryOp, Expr, ExprKind};

impl Compiler {
    pub fn compile_assign(&mut self, name: &str, value: &Expr, dest: u8, span: Span) -> Result<()> {
        if let Some((reg, mutable)) = self.resolve_variable(name) {
            if self.loop_variables.contains(&name.to_string()) {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToLoopVariable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }

            if !mutable {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
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
                if let (ExprKind::Identifier(id), ExprKind::Int(n)) = (&left.kind, &right.kind)
                    && id == name
                    && *n >= 0
                    && *n <= 255
                {
                    self.emit_a(OpCode::AddI, reg, reg, *n as u8, span);
                    if reg != dest {
                        self.emit_a(OpCode::Move, dest, reg, 0, span);
                    }
                    return Ok(());
                }
                if let (ExprKind::Int(n), ExprKind::Identifier(id)) = (&left.kind, &right.kind)
                    && id == name
                    && *n >= 0
                    && *n <= 255
                {
                    self.emit_a(OpCode::AddI, reg, reg, *n as u8, span);
                    if reg != dest {
                        self.emit_a(OpCode::Move, dest, reg, 0, span);
                    }
                    return Ok(());
                }
            }

            if let ExprKind::Binary {
                left,
                op: BinaryOp::Sub,
                right,
            } = &value.kind
                && let (ExprKind::Identifier(id), ExprKind::Int(n)) = (&left.kind, &right.kind)
                && id == name
                && *n >= 0
                && *n <= 255
            {
                self.emit_a(OpCode::SubI, reg, reg, *n as u8, span);
                if reg != dest {
                    self.emit_a(OpCode::Move, dest, reg, 0, span);
                }
                return Ok(());
            }

            self.compile_expr(value, reg)?;
            if reg != dest {
                self.emit_a(OpCode::Move, dest, reg, 0, span);
            }

            Ok(())
        } else if let Some((upvalue_idx, mutable)) = self.resolve_upvalue(name) {
            if !mutable {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }

            self.compile_expr(value, dest)?;
            self.emit_a(OpCode::SetUpval, upvalue_idx, dest, 0, span);

            Ok(())
        } else if let Some(&mutable) = self.globals.get(name) {
            if !mutable {
                return Err(CompileError::new(
                    CompileErrorKind::AssignToImmutable(name.to_string()),
                    span,
                    self.source.clone(),
                )
                .into());
            }

            if !self.global_indices.contains_key(name)
                && let ExprKind::Binary {
                    left,
                    op: BinaryOp::Add,
                    right,
                } = &value.kind
            {
                if let (ExprKind::Identifier(id), ExprKind::Int(n)) = (&left.kind, &right.kind)
                    && id == name
                    && *n >= 0
                    && *n <= 255
                {
                    let name_ref = self.heap.intern_string(name);
                    let const_idx = self.add_constant(Value::ptr(name_ref.index()), span)?;
                    self.emit_c(OpCode::IncGlobalI, dest, const_idx as u8, *n as u8, span);
                    return Ok(());
                }
                if let (ExprKind::Int(n), ExprKind::Identifier(id)) = (&left.kind, &right.kind)
                    && id == name
                    && *n >= 0
                    && *n <= 255
                {
                    let name_ref = self.heap.intern_string(name);
                    let const_idx = self.add_constant(Value::ptr(name_ref.index()), span)?;
                    self.emit_c(OpCode::IncGlobalI, dest, const_idx as u8, *n as u8, span);
                    return Ok(());
                }
            }

            self.compile_expr(value, dest)?;

            let idx = if let Some(&idx) = self.global_indices.get(name) {
                idx
            } else {
                let idx = self.next_global_index;
                self.global_indices.insert(name.to_string(), idx);
                self.next_global_index += 1;
                idx
            };
            self.accessed_globals.insert(name.to_string());
            self.emit_b(OpCode::SetGlobalIdx, dest, idx as i16, span);

            Ok(())
        } else {
            Err(CompileError::new(
                CompileErrorKind::UndefinedVariable(name.to_string()),
                span,
                self.source.clone(),
            )
            .into())
        }
    }
}
