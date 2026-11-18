use super::LoweringContext;
use crate::*;
use aelys_sema::{InferType, TypedExpr, TypedExprKind, TypedStmt};
use aelys_syntax::UnaryOp;

impl<'a> LoweringContext<'a> {
    fn assert_step(&mut self, step: LocalId, op: BinOp, zero: Operand, message: &str, sp: Option<Span>) {
        let holds = self.emit_rvalue_to_temp(AirType::Bool, Rvalue::BinaryOp(op, Operand::Copy(step), zero), sp);
        let ok = self.alloc_block_id();
        let fail = self.alloc_block_id();
        self.seal_block(AirTerminator::Branch {
            cond: holds,
            then_block: ok,
            else_block: fail,
        });
        self.fixup_block_id_noop(fail);
        self.seal_block(AirTerminator::Panic {
            message: message.to_string(),
            span: sp,
        });
        self.fixup_block_id_noop(ok);
    }

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

        let before = self.str_temp_ids();
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
        if let Some(step_expr) = step.as_ref().filter(|s| !step_expr_is_positive(s)) {
            let at = Some(self.span(&step_expr.span));
            let zero = Operand::Const(
                iter_ty
                    .int_size()
                    .map(|s| AirConst::Int(0, s))
                    .unwrap_or(AirConst::IntLiteral(0)),
            );
            self.assert_step(step_local, BinOp::Ne, zero.clone(), "assertion failed: step != 0", at);
            self.assert_step(step_local, BinOp::Gt, zero, "assertion failed: step > 0", at);
        }

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.release_arm_str_temps(&before);
        let dominated = self.str_temps_posed_here();
        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let cond_local = self.alloc_temp(AirType::Bool);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(cond_local),
                rvalue: Rvalue::BinaryOp(
                    if inclusive { BinOp::Le } else { BinOp::Lt },
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
            body_scope_depth: self.locals_by_name.len(),
            str_temps_at_entry: self.str_temp_ids(),
            views_at_entry: (self.views.captures.len(), self.views.held.len()),
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
        self.restamp_str_temps(&dominated);
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
        let before = self.str_temp_ids();
        let held = self.view_hold(iterable);
        let mut collection = self.lower_expr(iterable);
        self.view_hold_done(held);
        let col_ty = self.lower_type_from_infer(&iterable.ty);
        let mut owned_col = None;
        // a stable binding cannot be written by the body, so the loop reads it in place
        let stable_binding = match &collection {
            Operand::Copy(id) | Operand::Move(id) if self.counted_air(&col_ty) => {
                self.str_binding_is_stable(*id).then_some(*id)
            }
            Operand::Copy(_) | Operand::Move(_) | Operand::Const(_) => None,
        };
        let col_local = match stable_binding {
            Some(id) => id,
            None => {
                let str_init = self
                    .counted_air(&col_ty)
                    .then(|| self.claim_str_init(&mut collection));
                let col_local = self.alloc_temp(col_ty.clone());
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(col_local),
                        rvalue: Rvalue::Use(collection),
                    },
                    sp,
                );
                if let Some(kind) = str_init {
                    if !matches!(kind, super::StrInit::Own) {
                        self.emit_str_retain(col_local, sp);
                    }
                    owned_col = Some(col_local);
                }
                col_local
            }
        };

        let idx_local = self.alloc_temp_mut(AirType::I64);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(idx_local),
                rvalue: Rvalue::Use(Operand::Const(AirConst::IntLiteral(0))),
            },
            None,
        );

        // a string carries a byte cursor beside the index so the traversal decodes each character once
        let byte_local = matches!(col_ty, AirType::Str).then(|| {
            let cursor = self.alloc_temp_mut(AirType::I64);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(cursor),
                    rvalue: Rvalue::Use(Operand::Const(AirConst::IntLiteral(0))),
                },
                None,
            );
            cursor
        });

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
        if let Some(col) = owned_col {
            self.str_locals.push((col, scope_depth + 1));
        }

        let elem_air_ty = self.lower_type_from_infer(elem_type);
        let elem_local = self.alloc_named_local(iterator, elem_air_ty.clone(), false, sp);
        let borrows_collection = owned_col.is_some() || stable_binding.is_some();
        let elem_owned =
            self.counted_air(&elem_air_ty) && col_ty != AirType::Str && !borrows_collection;

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.release_arm_str_temps(&before);
        let dominated = self.str_temps_posed_here();
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
        let mut width_local = None;
        let elem_rvalue = if let Some(cursor) = byte_local {
            let packed = self.alloc_temp(AirType::I64);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(packed),
                    rvalue: Rvalue::Call {
                        func: Callee::Named("__aelys_str_decode_at".to_string()),
                        args: vec![Operand::Copy(col_local), Operand::Copy(cursor)],
                    },
                },
                None,
            );
            let width = self.alloc_temp(AirType::I64);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(width),
                    rvalue: Rvalue::BinaryOp(
                        BinOp::Shr,
                        Operand::Copy(packed),
                        Operand::Const(AirConst::IntLiteral(32)),
                    ),
                },
                None,
            );
            width_local = Some(width);
            Rvalue::Cast {
                operand: Operand::Copy(packed),
                from: AirType::I64,
                to: elem_air_ty,
            }
        } else {
            Rvalue::Index {
                base: Operand::Copy(col_local),
                index: Operand::Copy(idx_local),
            }
        };
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(elem_local),
                rvalue: elem_rvalue,
            },
            None,
        );
        if elem_owned {
            self.emit_str_retain(elem_local, None);
            self.str_locals.push((elem_local, scope_depth + 1));
        }

        // the element is rebound every turn, so continue and break must release it like a body local
        let body_scope_depth = if elem_owned {
            scope_depth
        } else {
            self.locals_by_name.len()
        };
        self.loop_stack.push(super::LoopBlocks {
            header: incr_id,
            exit: exit_id,
            body_scope_depth,
            str_temps_at_entry: self.str_temp_ids(),
            views_at_entry: (self.views.captures.len(), self.views.held.len()),
        });
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            if elem_owned {
                self.emit_str_release(elem_local, None);
            }
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.loop_stack.pop();
        if elem_owned {
            self.forget_str_local(elem_local);
        }

        self.fixup_block_id_noop(incr_id);
        if let (Some(cursor), Some(width)) = (byte_local, width_local) {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(cursor),
                    rvalue: Rvalue::BinaryOp(
                        BinOp::Add,
                        Operand::Copy(cursor),
                        Operand::Copy(width),
                    ),
                },
                None,
            );
        }
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
        self.restamp_str_temps(&dominated);
        self.emit_scope_rc_releases(scope_depth);
        self.locals_by_name.truncate(scope_depth);
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
