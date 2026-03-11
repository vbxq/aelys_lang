use super::{LoweringContext, infer_to_int_size, lower_binop, lower_unop};
use crate::*;
use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedMatchArm, TypedParam,
    TypedPattern, TypedStmt,
};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_expr(&mut self, expr: &TypedExpr) -> Operand {
        let sp = Some(self.span(&expr.span));
        match &expr.kind {
            TypedExprKind::Int(v) => {
                if expr.ty.is_integer() {
                    Operand::Const(AirConst::Int(*v, infer_to_int_size(&expr.ty)))
                } else {
                    Operand::Const(AirConst::IntLiteral(*v))
                }
            }
            TypedExprKind::Float(v) => {
                let size = if matches!(expr.ty, InferType::F32) {
                    AirFloatSize::F32
                } else {
                    AirFloatSize::F64
                };
                Operand::Const(AirConst::Float(*v, size))
            }
            TypedExprKind::Bool(v) => Operand::Const(AirConst::Bool(*v)),
            TypedExprKind::String(v) => Operand::Const(AirConst::Str(v.clone())),
            TypedExprKind::Null => Operand::Const(AirConst::Null),

            TypedExprKind::Identifier(name) => {
                if let Some(id) = self.lookup_local(name) {
                    Operand::Copy(id)
                } else if self.globals.iter().any(|global| global.name == *name) {
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&expr.ty),
                        Rvalue::Call {
                            func: Callee::Named(format!("__aelys_global_get_{}", name)),
                            args: Vec::new(),
                        },
                        sp,
                    )
                } else if matches!(expr.ty, InferType::Function { .. }) {
                    // Keep named functions in a typed temp when they are used as values.
                    // A raw FnRef constant looks like ptr<void> to later mono passes.
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&expr.ty),
                        Rvalue::Use(Operand::Const(AirConst::FnRef(name.clone()))),
                        sp,
                    )
                } else {
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&expr.ty),
                        Rvalue::Call {
                            func: Callee::Named(format!("__aelys_global_get_{}", name)),
                            args: Vec::new(),
                        },
                        sp,
                    )
                }
            }

            TypedExprKind::Binary { left, op, right } => {
                let l = self.lower_expr(left);
                let r = self.lower_expr(right);
                let air_op = lower_binop(op);
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::BinaryOp(air_op, l, r),
                    sp,
                )
            }

            TypedExprKind::Unary { op, operand } => {
                let inner = self.lower_expr(operand);
                let air_op = lower_unop(op);
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::UnaryOp(air_op, inner),
                    sp,
                )
            }

            TypedExprKind::And { left, right } => self.lower_short_circuit(left, right, true, expr),

            TypedExprKind::Or { left, right } => self.lower_short_circuit(left, right, false, expr),

            TypedExprKind::Call { callee, args } => {
                let lowered_args: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let func = self.lower_callee(callee);
                self.lower_call_common(func, lowered_args, &expr.ty, sp)
            }

            TypedExprKind::Assign { name, value } => self.lower_assign_common(name, value, sp),

            TypedExprKind::Grouping(inner) => self.lower_expr(inner),

            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if_expr(condition, then_branch, else_branch, expr),

            TypedExprKind::Lambda(inner) => self.lower_expr(inner),

            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => self.lower_lambda(params, return_type, body, captures, expr),

            TypedExprKind::FmtString(parts) => self.lower_fmt_string(parts, sp),

            TypedExprKind::Member { object, member } => {
                let base = self.lower_expr(object);
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::FieldAccess {
                        base,
                        field: member.clone(),
                    },
                    sp,
                )
            }

            TypedExprKind::StructLiteral { name, fields } => {
                let lowered_fields: Vec<(String, Operand)> = fields
                    .iter()
                    .map(|(fname, fval)| (fname.clone(), self.lower_expr(fval)))
                    .collect();
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::StructInit {
                        name: name.clone(),
                        fields: lowered_fields,
                    },
                    sp,
                )
            }

            TypedExprKind::ArrayLiteral { elements, .. } => {
                let lowered: Vec<Operand> = elements.iter().map(|e| self.lower_expr(e)).collect();
                let n = lowered.len() as u64;
                // extract element type from the array type
                let elem_ty = match &expr.ty {
                    InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                    _ => AirType::I64,
                };
                let arr_ty = AirType::Array(Box::new(elem_ty), n);
                let arr_local = self.alloc_temp_mut(arr_ty);
                for (i, elem_op) in lowered.into_iter().enumerate() {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Index(
                                arr_local,
                                Operand::Const(AirConst::IntLiteral(i as i64)),
                            ),
                            rvalue: Rvalue::Use(elem_op),
                        },
                        sp,
                    );
                }
                Operand::Copy(arr_local)
            }

            TypedExprKind::ArraySized {
                size, fill_value, ..
            } => {
                // extract the const size (no longer panics on non-constant)
                let n = match &size.kind {
                    TypedExprKind::Int(v) => *v as u64,
                    _ => {
                        self.report_error(
                            "unsupported non-constant array size in AIR lowering: \
                             ArraySized requires a constant integer size expression"
                                .to_string(),
                        );
                        // Fallback: treat as zero-length array so lowering can continue
                        0
                    }
                };
                let elem_ty = match &expr.ty {
                    InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                    _ => AirType::I64,
                };
                self.check_stack_array_size(&elem_ty, n);
                let arr_ty = AirType::Array(Box::new(elem_ty), n);
                let arr_local = self.alloc_temp_mut(arr_ty);
                // lower fill value or use zero-init
                let fill_op = if let Some(fv) = fill_value {
                    self.lower_expr(fv)
                } else {
                    let elem_ty_for_zero = match &expr.ty {
                        InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                        _ => AirType::I64,
                    };
                    Operand::Const(AirConst::ZeroInit(elem_ty_for_zero))
                };
                for i in 0..n {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Index(
                                arr_local,
                                Operand::Const(AirConst::IntLiteral(i as i64)),
                            ),
                            rvalue: Rvalue::Use(fill_op.clone()),
                        },
                        sp,
                    );
                }
                Operand::Copy(arr_local)
            }

            TypedExprKind::VecLiteral { elements, .. } => {
                let lowered: Vec<Operand> = elements.iter().map(|e| self.lower_expr(e)).collect();
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::Call {
                        func: Callee::Named("__aelys_vec_new".to_string()),
                        args: lowered,
                    },
                    sp,
                )
            }

            TypedExprKind::Index { object, index } => {
                let obj = self.lower_expr(object);
                let idx = self.lower_expr(index);
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::Index {
                        base: obj,
                        index: idx,
                    },
                    sp,
                )
            }

            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => self.lower_index_assign(object, index, value, sp),

            TypedExprKind::Range { start, end, .. } => {
                let mut args = Vec::new();
                if let Some(s) = start {
                    args.push(self.lower_expr(s));
                }
                if let Some(e) = end {
                    args.push(self.lower_expr(e));
                }
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::Call {
                        func: Callee::Named("__aelys_range".to_string()),
                        args,
                    },
                    sp,
                )
            }

            TypedExprKind::Slice { object, range } => {
                let obj = self.lower_expr(object);
                let rng = self.lower_expr(range);
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::Call {
                        func: Callee::Named("__aelys_slice".to_string()),
                        args: vec![obj, rng],
                    },
                    sp,
                )
            }

            TypedExprKind::Cast {
                expr: inner,
                target,
            } => {
                let operand = self.lower_expr(inner);
                let from = self.lower_type_from_infer(&inner.ty);
                let to = self.lower_type_from_infer(target);
                self.emit_rvalue_to_temp(to.clone(), Rvalue::Cast { operand, from, to }, sp)
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                tag,
                args,
            } => {
                let payload: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&expr.ty),
                    Rvalue::EnumInit {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        tag: *tag,
                        payload,
                    },
                    sp,
                )
            }
            TypedExprKind::Block { stmts, tail } => {
                for stmt in stmts {
                    self.lower_stmt(stmt);
                }
                self.lower_expr(tail)
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.lower_match_expr(scrutinee, arms, expr)
            }
        }
    }

    pub(super) fn lower_expr_discard(&mut self, expr: &TypedExpr) {
        let sp = Some(self.span(&expr.span));
        match &expr.kind {
            TypedExprKind::Call { callee, args } => {
                let lowered_args: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let func = self.lower_callee(callee);
                let ret_ty = self.lower_type_from_infer(&expr.ty);
                // Void and Opaque calls in discard position should emit CallVoid.
                //
                // Opaque means the return type is unresolved Dynamic. Since the caller is discarding the result anyway,
                // there's no point creating a temp local with an unresolvable type.
                if matches!(ret_ty, AirType::Void | AirType::Opaque) {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func,
                            args: lowered_args,
                        },
                        sp,
                    );
                } else {
                    self.emit_rvalue_to_temp(
                        ret_ty,
                        Rvalue::Call {
                            func,
                            args: lowered_args,
                        },
                        sp,
                    );
                }
            }
            TypedExprKind::Assign { name, value } => {
                self.lower_assign_common(name, value, sp);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.lower_index_assign(object, index, value, sp);
            }
            _ => {
                self.lower_expr(expr);
            }
        }
    }

    /// Shared logic for Call expressions: emit CallVoid for void, otherwise assign to temp.
    fn lower_call_common(
        &mut self,
        func: Callee,
        args: Vec<Operand>,
        result_infer_ty: &InferType,
        sp: Option<Span>,
    ) -> Operand {
        let result_ty = self.lower_type_from_infer(result_infer_ty);
        // Void, and Opaque-returning calls can't be used as values in LLVM
        // Opaque means the return type is unresolved Dynamic (bootstrap builtins like print/println). Treating it as void prevents creating temp locals with an unresolvable type.
        // TODO: !
        if matches!(result_ty, AirType::Void | AirType::Opaque) {
            self.emit(AirStmtKind::CallVoid { func, args }, sp);
            Operand::Const(AirConst::Null)
        } else {
            self.emit_rvalue_to_temp(result_ty, Rvalue::Call { func, args }, sp)
        }
    }

    /// Shared logic for Assign expressions.
    fn lower_assign_common(&mut self, name: &str, value: &TypedExpr, sp: Option<Span>) -> Operand {
        let val = self.lower_expr(value);
        if let Some(id) = self.lookup_local(name) {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(id),
                    rvalue: Rvalue::Use(val),
                },
                sp,
            );
            Operand::Copy(id)
        } else {
            self.emit(
                AirStmtKind::CallVoid {
                    func: Callee::Named(format!("__aelys_global_set_{}", name)),
                    args: vec![val],
                },
                sp,
            );
            Operand::Const(AirConst::Null)
        }
    }

    /// Shared logic for IndexAssign expressions.
    fn lower_index_assign(
        &mut self,
        object: &TypedExpr,
        index: &TypedExpr,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        let obj = self.lower_expr(object);
        let idx = self.lower_expr(index);
        let val = self.lower_expr(value);
        let obj_ty = self.lower_type_from_infer(&object.ty);
        let base_local = self.operand_to_local(obj, &obj_ty);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Index(base_local, idx),
                rvalue: Rvalue::Use(val),
            },
            sp,
        );
        Operand::Const(AirConst::Null)
    }

    fn lower_callee(&mut self, callee: &TypedExpr) -> Callee {
        match &callee.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(id) = self.lookup_local(name) {
                    Callee::FnPtr(id)
                } else if self.globals.iter().any(|global| global.name == *name) {
                    // A callable file-scope let is still data in global storage; lower the
                    // callee through the global getter so calls stay indirect.
                    let op = self.lower_expr(callee);
                    let ty = self.lower_type_from_infer(&callee.ty);
                    Callee::FnPtr(self.operand_to_local(op, &ty))
                } else {
                    Callee::Named(name.clone())
                }
            }
            TypedExprKind::Member { object, member } => {
                if let TypedExprKind::Identifier(mod_name) = &object.kind {
                    let is_runtime_value = self.lookup_local(mod_name).is_some()
                        || self.globals.iter().any(|global| global.name == *mod_name);
                    if !is_runtime_value {
                        Callee::Named(format!("{}.{}", mod_name, member))
                    } else {
                        // `value.field()` on a struct/global fnptr field must stay indirect.
                        let op = self.lower_expr(callee);
                        let ty = self.lower_type_from_infer(&callee.ty);
                        Callee::FnPtr(self.operand_to_local(op, &ty))
                    }
                } else {
                    let op = self.lower_expr(callee);
                    let ty = self.lower_type_from_infer(&callee.ty);
                    Callee::FnPtr(self.operand_to_local(op, &ty))
                }
            }
            _ => {
                let op = self.lower_expr(callee);
                let ty = self.lower_type_from_infer(&callee.ty);
                Callee::FnPtr(self.operand_to_local(op, &ty))
            }
        }
    }

    // short-circuit lowering (and/or)
    fn lower_short_circuit(
        &mut self,
        left: &TypedExpr,
        right: &TypedExpr,
        is_and: bool,
        _parent: &TypedExpr,
    ) -> Operand {
        let result = self.alloc_temp_mut(AirType::Bool);
        let lhs = self.lower_expr(left);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(lhs),
            },
            None,
        );

        let eval_right_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        if is_and {
            self.seal_block(AirTerminator::Branch {
                cond: Operand::Copy(result),
                then_block: eval_right_id,
                else_block: merge_id,
            });
        } else {
            self.seal_block(AirTerminator::Branch {
                cond: Operand::Copy(result),
                then_block: merge_id,
                else_block: eval_right_id,
            });
        }

        self.fixup_block_id_noop(eval_right_id);
        let rhs = self.lower_expr(right);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(rhs),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(merge_id);
        Operand::Copy(result)
    }

    fn lower_if_expr(
        &mut self,
        condition: &TypedExpr,
        then_branch: &TypedExpr,
        else_branch: &TypedExpr,
        parent: &TypedExpr,
    ) -> Operand {
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let result = self.alloc_temp_mut(result_ty);

        let cond = self.lower_expr(condition);
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: then_id,
            else_block: else_id,
        });

        self.fixup_block_id_noop(then_id);
        let then_val = self.lower_expr(then_branch);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(then_val),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(else_id);
        let else_val = self.lower_expr(else_branch);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(else_val),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(merge_id);
        Operand::Copy(result)
    }

    // lambda lowering (desugared to closure env struct + function)
    fn lower_lambda(
        &mut self,
        params: &[TypedParam],
        return_type: &InferType,
        body: &[TypedStmt],
        captures: &[(String, InferType)],
        parent: &TypedExpr,
    ) -> Operand {
        use aelys_sema::TypedFunction;

        let lambda_name = format!("__lambda_{}", self.next_function_id);
        let fake_func = TypedFunction {
            name: lambda_name.clone(),
            type_params: Vec::new(),
            params: params.to_vec(),
            return_type: return_type.clone(),
            body: body.to_vec(),
            decorators: Vec::new(),
            is_pub: false,
            span: parent.span,
            captures: captures.to_vec(),
        };
        self.lower_function(&fake_func);

        self.emit_rvalue_to_temp(
            self.lower_type_from_infer(&parent.ty),
            Rvalue::Use(Operand::Const(AirConst::FnRef(lambda_name))),
            Some(self.span(&parent.span)),
        )
    }

    fn lower_match_expr(
        &mut self,
        scrutinee: &TypedExpr,
        arms: &[TypedMatchArm],
        parent: &TypedExpr,
    ) -> Operand {
        let sp = Some(self.span(&parent.span));
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let result = self.alloc_temp_mut(result_ty);

        // Lower the scrutinee
        let scrutinee_op = self.lower_expr(scrutinee);

        // Get the enum name from the scrutinee type
        let enum_name = match &scrutinee.ty {
            InferType::Enum(name, _) => name.clone(),
            _ => {
                self.report_error(format!(
                    "match scrutinee is not an enum type: {:?}",
                    scrutinee.ty
                ));
                return Operand::Const(AirConst::Null);
            }
        };

        // Extract the tag
        let tag_op = self.emit_rvalue_to_temp(
            AirType::I32,
            Rvalue::EnumTag {
                enum_name: enum_name.clone(),
                operand: scrutinee_op.clone(),
            },
            sp,
        );

        // Allocate blocks: one per arm + merge block
        let merge_id = self.alloc_block_id();

        // Separate variant arms from wildcard
        let mut switch_targets: Vec<(AirConst, BlockId)> = Vec::new();
        let mut wildcard_arm: Option<&TypedMatchArm> = None;
        let mut arm_blocks: Vec<(BlockId, &TypedMatchArm)> = Vec::new();

        for arm in arms {
            match &arm.pattern {
                TypedPattern::Variant { tag, .. } => {
                    let block_id = self.alloc_block_id();
                    switch_targets.push((AirConst::Int(*tag as i64, AirIntSize::I32), block_id));
                    arm_blocks.push((block_id, arm));
                }
                TypedPattern::Wildcard => {
                    wildcard_arm = Some(arm);
                }
            }
        }

        // Allocate a default block for the wildcard (or unreachable if exhaustive)
        let default_id = self.alloc_block_id();

        // Seal current block with Switch terminator
        self.seal_block(AirTerminator::Switch {
            discr: tag_op,
            targets: switch_targets,
            default: default_id,
        });

        // Lower each variant arm
        for (block_id, arm) in arm_blocks {
            self.fixup_block_id_noop(block_id);

            // Bind payload fields if the pattern has bindings
            if let TypedPattern::Variant {
                enum_name: arm_enum_name,
                tag,
                bindings,
                ..
            } = &arm.pattern
            {
                for (field_index, (name, ty)) in bindings.iter().enumerate() {
                    let field_ty = self.lower_type_from_infer(ty);
                    let field_local = self.alloc_named_local(name, field_ty.clone(), false, sp);
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(field_local),
                            rvalue: Rvalue::EnumPayload {
                                enum_name: arm_enum_name.clone(),
                                tag: *tag,
                                operand: scrutinee_op.clone(),
                                field_index: field_index as u32,
                            },
                        },
                        sp,
                    );
                }
            }

            let arm_val = self.lower_expr(&arm.body);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(result),
                    rvalue: Rvalue::Use(arm_val),
                },
                None,
            );
            self.seal_block(AirTerminator::Goto(merge_id));
        }

        // Lower the default block (wildcard or unreachable)
        self.fixup_block_id_noop(default_id);
        if let Some(wildcard) = wildcard_arm {
            let wc_val = self.lower_expr(&wildcard.body);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(result),
                    rvalue: Rvalue::Use(wc_val),
                },
                None,
            );
            self.seal_block(AirTerminator::Goto(merge_id));
        } else {
            // Exhaustive match without wildcard -- all variants covered, default is unreachable
            self.seal_block(AirTerminator::Unreachable);
        }

        self.fixup_block_id_noop(merge_id);
        Operand::Copy(result)
    }

    // format string -> __aelys_str_concat / __aelys_to_string
    fn lower_fmt_string(&mut self, parts: &[TypedFmtStringPart], sp: Option<Span>) -> Operand {
        let mut operands: Vec<Operand> = Vec::new();

        for part in parts {
            match part {
                TypedFmtStringPart::Literal(s) => {
                    operands.push(Operand::Const(AirConst::Str(s.clone())));
                }
                TypedFmtStringPart::Expr(expr) => {
                    let val = self.lower_expr(expr);
                    if matches!(expr.ty, InferType::String) {
                        operands.push(val);
                    } else {
                        let converted = self.emit_rvalue_to_temp(
                            AirType::Str,
                            Rvalue::Call {
                                func: Callee::Named("__aelys_to_string".to_string()),
                                args: vec![val],
                            },
                            None,
                        );
                        operands.push(converted);
                    }
                }
                TypedFmtStringPart::Placeholder => {
                    operands.push(Operand::Const(AirConst::Str(String::new())));
                }
            }
        }

        if operands.is_empty() {
            return Operand::Const(AirConst::Str(String::new()));
        }
        if operands.len() == 1 {
            return operands.into_iter().next().unwrap();
        }

        let mut acc = operands.remove(0);
        for part in operands {
            acc = self.emit_rvalue_to_temp(
                AirType::Str,
                Rvalue::Call {
                    func: Callee::Named("__aelys_str_concat".to_string()),
                    args: vec![acc, part],
                },
                sp,
            );
        }
        acc
    }
}
