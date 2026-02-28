use super::LoweringContext;
use crate::*;
use aelys_sema::{InferType, TypedExpr, TypedStmt};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_for(
        &mut self,
        iterator: &str,
        start: &TypedExpr,
        end: &TypedExpr,
        inclusive: bool,
        step: &Option<TypedExpr>,
        body: &TypedStmt,
    ) {
        let start_span = Some(self.span(&start.span));
        let iter_ty = self.lower_type_from_infer(&start.ty);
        let iter_local = self.alloc_named_local(iterator, iter_ty.clone(), true, start_span);
        let start_op = self.lower_expr(start);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::Use(start_op),
            },
            start_span,
        );

        let end_local = self.alloc_temp(iter_ty.clone());
        let end_op = self.lower_expr(end);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(end_local),
                rvalue: Rvalue::Use(end_op),
            },
            Some(self.span(&end.span)),
        );

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let cmp_op = if inclusive { BinOp::Le } else { BinOp::Lt };
        let cond_local = self.alloc_temp(AirType::Bool);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(cond_local),
                rvalue: Rvalue::BinaryOp(
                    cmp_op,
                    Operand::Copy(iter_local),
                    Operand::Copy(end_local),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Branch {
            cond: Operand::Copy(cond_local),
            then_block: body_id,
            else_block: exit_id,
        });

        self.loop_stack.push(super::LoopBlocks {
            header: incr_id,
            exit: exit_id,
        });
        self.fixup_block_id_noop(body_id);
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(incr_id);
        // S1: was always IntLiteral(1) aka i64. Blows up on i32/i16/i8 iterators.
        let step_operand = if let Some(step_expr) = step {
            self.lower_expr(step_expr)
        } else {
            let step = iter_ty
                .int_size()
                .map(|s| AirConst::Int(1, s))
                .unwrap_or(AirConst::IntLiteral(1));
            Operand::Const(step)
        };
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::BinaryOp(BinOp::Add, Operand::Copy(iter_local), step_operand),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(exit_id);
    }

    pub(super) fn lower_foreach(
        &mut self,
        iterator: &str,
        iterable: &TypedExpr,
        elem_type: &InferType,
        body: &TypedStmt,
        sp: Option<Span>,
    ) {
        let collection = self.lower_expr(iterable);
        let col_ty = self.lower_type_from_infer(&iterable.ty);
        let col_local = self.alloc_temp(col_ty.clone());
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(col_local),
                rvalue: Rvalue::Use(collection),
            },
            sp,
        );

        let idx_local = self.alloc_temp_mut(AirType::I64);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(idx_local),
                rvalue: Rvalue::Use(Operand::Const(AirConst::IntLiteral(0))),
            },
            None,
        );

        let len_local = self.alloc_temp(AirType::I64);
        // for stack arrays with known length, use the constant directly
        let len_rvalue = if let AirType::Array(_, n) = &col_ty {
            Rvalue::Use(Operand::Const(AirConst::IntLiteral(*n as i64)))
        } else {
            Rvalue::Call {
                func: Callee::Named("__aelys_len".to_string()),
                args: vec![Operand::Copy(col_local)],
            }
        };
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(len_local),
                rvalue: len_rvalue,
            },
            None,
        );

        let elem_air_ty = self.lower_type_from_infer(elem_type);
        let elem_local = self.alloc_named_local(iterator, elem_air_ty, false, sp);

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));
        self.fixup_block_id_noop(header_id);

        let cond_local = self.alloc_temp(AirType::Bool);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(cond_local),
                rvalue: Rvalue::BinaryOp(
                    BinOp::Lt,
                    Operand::Copy(idx_local),
                    Operand::Copy(len_local),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Branch {
            cond: Operand::Copy(cond_local),
            then_block: body_id,
            else_block: exit_id,
        });

        self.fixup_block_id_noop(body_id);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(elem_local),
                rvalue: Rvalue::Index {
                    base: Operand::Copy(col_local),
                    index: Operand::Copy(idx_local),
                },
            },
            None,
        );

        self.loop_stack.push(super::LoopBlocks {
            header: incr_id,
            exit: exit_id,
        });
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(incr_id);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(idx_local),
                rvalue: Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(idx_local),
                    Operand::Const(AirConst::IntLiteral(1)),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(exit_id);
    }
}
