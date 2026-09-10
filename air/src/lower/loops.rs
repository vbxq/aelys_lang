use super::LoweringContext;
use crate::*;
use aelys_sema::{InferType, TypedExpr, TypedExprKind, TypedStmt};
use aelys_syntax::UnaryOp;

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

        // evaluate range bounds before allocating the iterator local so that
        let start_op = self.lower_expr(start);
        let end_op = self.lower_expr(end);

        // save scope so the iterator variable doesn't leak into the enclosing
        let scope_depth = self.locals_by_name.len();

        let iter_local = self.alloc_named_local(iterator, iter_ty.clone(), true, start_span);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::Use(start_op),
            },
            start_span,
        );

        let end_local = self.alloc_temp(iter_ty.clone());
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(end_local),
                rvalue: Rvalue::Use(end_op),
            },
            Some(self.span(&end.span)),
        );

        let step_operand = if let Some(step_expr) = step {
            self.lower_expr(step_expr)
        } else {
            let step_c = iter_ty
                .int_size()
                .map(|s| AirConst::Int(1, s))
                .unwrap_or(AirConst::IntLiteral(1));
            Operand::Const(step_c)
        };
        let step_local = self.alloc_temp(iter_ty.clone());
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(step_local),
                rvalue: Rvalue::Use(step_operand),
            },
            None,
        );

        if self.position_is_dead() {
            self.locals_by_name.truncate(scope_depth);
            return;
        }

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let step_is_negative = step.as_ref().is_some_and(|s| step_expr_is_negative(s));
        let step_is_const = step.as_ref().map_or(true, |s| {
            step_expr_is_negative(s) || step_expr_is_positive(s)
        });

        let cond_local = self.alloc_temp(AirType::Bool);
        if step_is_const {
            let cmp_op = if step_is_negative {
                if inclusive { BinOp::Ge } else { BinOp::Gt }
            } else {
                if inclusive { BinOp::Le } else { BinOp::Lt }
            };
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
        } else {
            let zero = Operand::Const(
                iter_ty
                    .int_size()
                    .map(|s| AirConst::Int(0, s))
                    .unwrap_or(AirConst::IntLiteral(0)),
            );
            let step_neg_local = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(step_neg_local),
                    rvalue: Rvalue::BinaryOp(BinOp::Lt, Operand::Copy(step_local), zero),
                },
                None,
            );
            let fwd_op = if inclusive { BinOp::Le } else { BinOp::Lt };
            let bwd_op = if inclusive { BinOp::Ge } else { BinOp::Gt };
            let fwd_local = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(fwd_local),
                    rvalue: Rvalue::BinaryOp(
                        fwd_op,
                        Operand::Copy(iter_local),
                        Operand::Copy(end_local),
                    ),
                },
                None,
            );
            let bwd_local = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(bwd_local),
                    rvalue: Rvalue::BinaryOp(
                        bwd_op,
                        Operand::Copy(iter_local),
                        Operand::Copy(end_local),
                    ),
                },
                None,
            );
            let neg_and_bwd = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(neg_and_bwd),
                    rvalue: Rvalue::BinaryOp(
                        BinOp::And,
                        Operand::Copy(step_neg_local),
                        Operand::Copy(bwd_local),
                    ),
                },
                None,
            );
            let not_neg = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(not_neg),
                    rvalue: Rvalue::UnaryOp(UnOp::Not, Operand::Copy(step_neg_local)),
                },
                None,
            );
            let pos_and_fwd = self.alloc_temp(AirType::Bool);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(pos_and_fwd),
                    rvalue: Rvalue::BinaryOp(
                        BinOp::And,
                        Operand::Copy(not_neg),
                        Operand::Copy(fwd_local),
                    ),
                },
                None,
            );
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(cond_local),
                    rvalue: Rvalue::BinaryOp(
                        BinOp::Or,
                        Operand::Copy(neg_and_bwd),
                        Operand::Copy(pos_and_fwd),
                    ),
                },
                None,
            );
        }
        self.seal_block(AirTerminator::Branch {
            cond: Operand::Copy(cond_local),
            then_block: body_id,
            else_block: exit_id,
        });

        self.loop_stack.push(super::LoopBlocks {
            header: incr_id,
            exit: exit_id,
            body_scope_depth: self.locals_by_name.len(),
        });
        self.fixup_block_id_noop(body_id);
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(incr_id);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(iter_local),
                    Operand::Copy(step_local),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(exit_id);
        // iterator locals are never rc today, this just keeps the registry honest
        self.emit_scope_rc_releases(scope_depth);
        self.locals_by_name.truncate(scope_depth);
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
        let len_rvalue = match &col_ty {
            AirType::Array(_, n) => Rvalue::Use(Operand::Const(AirConst::IntLiteral(*n as i64))),
            AirType::Str => Rvalue::Call {
                func: Callee::Named("__aelys_str_char_count".to_string()),
                args: vec![Operand::Copy(col_local)],
            },
            AirType::Slice(_) => Rvalue::Len(Operand::Copy(col_local)),
            _ => Rvalue::Call {
                func: Callee::Named("__aelys_len".to_string()),
                args: vec![Operand::Copy(col_local)],
            },
        };
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(len_local),
                rvalue: len_rvalue,
            },
            None,
        );

        if self.position_is_dead() {
            return;
        }

        // save scope so the iterator variable doesn't leak after the loop.
        let scope_depth = self.locals_by_name.len();

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
            body_scope_depth: self.locals_by_name.len(),
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
        self.emit_scope_rc_releases(scope_depth);
        self.locals_by_name.truncate(scope_depth);
    }
}

fn step_expr_is_negative(step: &TypedExpr) -> bool {
    match &step.kind {
        TypedExprKind::Int(v) => *v < 0,
        TypedExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => match &operand.kind {
            TypedExprKind::Int(v) => *v > 0,
            _ => false,
        },
        _ => false,
    }
}

fn step_expr_is_positive(step: &TypedExpr) -> bool {
    match &step.kind {
        TypedExprKind::Int(v) => *v > 0,
        TypedExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => match &operand.kind {
            TypedExprKind::Int(v) => *v < 0,
            _ => false,
        },
        _ => false,
    }
}
