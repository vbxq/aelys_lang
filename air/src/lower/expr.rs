use super::{LoweringContext, infer_to_int_size, lower_binop, lower_unop};
use crate::*;
use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedMatchArm, TypedParam,
    TypedPattern, TypedStmt,
};

impl<'a> LoweringContext<'a> {
    /// Returns true for types that have no runtime representation (void, null,
    /// opaque).  Used to skip result assignments in match/if-else branches.
    fn is_void_like(ty: &AirType) -> bool {
        matches!(ty, AirType::Void | AirType::Opaque)
            || matches!(ty, AirType::Ptr(inner) if matches!(inner.as_ref(), AirType::Void))
    }

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
                    // Named function used as a value: wrap in a fat pointer with
                    // null env. Same representation as a closure; see lower_lambda.
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&expr.ty),
                        Rvalue::ClosureCreate {
                            fn_name: name.clone(),
                            env: Operand::Const(AirConst::Null),
                        },
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
                    other => {
                        self.report_error(format!(
                            "ICE: array literal has non-array type `{}` at AIR lowering",
                            other
                        ));
                        AirType::I64
                    }
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
                    other => {
                        self.report_error(format!(
                            "ICE: ArraySized has non-array type `{}` at AIR lowering",
                            other
                        ));
                        AirType::I64
                    }
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
                        other => {
                            self.report_error(format!(
                                "ICE: ArraySized zero-init has non-array type `{}` at AIR lowering",
                                other
                            ));
                            AirType::I64
                        }
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

            TypedExprKind::FieldAssign {
                object,
                field,
                value,
            } => self.lower_field_assign(object, field, value, sp),

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
                let scope_depth = self.locals_by_name.len();
                for stmt in stmts {
                    self.lower_stmt(stmt);
                }
                let result = self.lower_expr(tail);
                self.locals_by_name.truncate(scope_depth);
                result
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
                // Void, Opaque, and Ptr(Void) calls in discard position should emit CallVoid.
                if Self::is_void_like(&ret_ty) {
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
            TypedExprKind::FieldAssign {
                object,
                field,
                value,
            } => {
                self.lower_field_assign(object, field, value, sp);
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
        // Void, Opaque, and Ptr(Void) (the null type from InferType::Null,
        // used by builtins like print/println) can't be used as values in
        // LLVM.  Emit CallVoid so codegen never tries to capture the result.
        if Self::is_void_like(&result_ty) {
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
            // If this variable is a closure capture, write the new value back
            // to the env struct so future calls see the updated value.
            if let Some(env_id) = self.closure_env_param {
                if self.closure_captures.contains(name) {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Field(env_id, name.to_string()),
                            rvalue: Rvalue::Use(Operand::Copy(id)),
                        },
                        sp,
                    );
                }
            }
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
    ///
    /// Handles `obj[i] = val` where `obj` may be a chain of field accesses
    /// (e.g. `buf.data[i] = val`). In that case a read-modify-write is needed:
    /// load the array from the parent struct(s), assign into the element, then
    /// store the array back up the chain.
    fn lower_index_assign(
        &mut self,
        object: &TypedExpr,
        index: &TypedExpr,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        let idx = self.lower_expr(index);
        let val = self.lower_expr(value);

        // Peel Member layers to find the root (the actual array/collection) and path.
        let mut segments: Vec<(String, AirType)> = Vec::new();
        let mut current = object;
        loop {
            match &current.kind {
                TypedExprKind::Member { object: inner, member } => {
                    let field_ty = self.lower_type_from_infer(&current.ty);
                    segments.push((member.clone(), field_ty));
                    current = inner;
                }
                _ => break,
            }
        }
        segments.reverse(); // shallowest → deepest

        if segments.is_empty() {
            // Detect nested index: `arr[i][j] = val` where `current` is `arr[i]`.
            // A read-modify-write is needed: load arr[i] into a temp, assign temp[j] = val,
            // then store the temp back to arr[i].
            if let TypedExprKind::Index { object: parent_arr, index: parent_idx_expr } =
                &current.kind
            {
                let parent_arr_op = self.lower_expr(parent_arr);
                let parent_arr_ty = self.lower_type_from_infer(&parent_arr.ty);
                let parent_arr_local = self.operand_to_local(parent_arr_op, &parent_arr_ty);
                let parent_idx = self.lower_expr(parent_idx_expr);

                let elem_ty = self.lower_type_from_infer(&current.ty);
                let elem_local = self.alloc_temp_mut(elem_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(elem_local),
                        rvalue: Rvalue::Index {
                            base: Operand::Copy(parent_arr_local),
                            index: parent_idx.clone(),
                        },
                    },
                    sp,
                );
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Index(elem_local, idx),
                        rvalue: Rvalue::Use(val),
                    },
                    sp,
                );
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Index(parent_arr_local, parent_idx),
                        rvalue: Rvalue::Use(Operand::Copy(elem_local)),
                    },
                    sp,
                );
                return Operand::Const(AirConst::Null);
            }

            // Simple case: the object is directly accessible.
            let root_name = if let TypedExprKind::Identifier(name) = &current.kind {
                Some(name.clone())
            } else {
                None
            };
            let obj = self.lower_expr(current);
            let obj_ty = self.lower_type_from_infer(&current.ty);
            let base_local = self.operand_to_local(obj, &obj_ty);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Index(base_local, idx),
                    rvalue: Rvalue::Use(val),
                },
                sp,
            );
            // Write the mutated array back to the closure env if it's a capture,
            // or back to the global store if the root is a global variable.
            if let Some(ref name) = root_name {
                if let Some(env_id) = self.closure_env_param {
                    if self.closure_captures.contains(name) {
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Field(env_id, name.clone()),
                                rvalue: Rvalue::Use(Operand::Copy(base_local)),
                            },
                            sp,
                        );
                    }
                }
                if self.lookup_local(name).is_none()
                    && self.globals.iter().any(|g| g.name == *name)
                {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func: Callee::Named(format!("__aelys_global_set_{}", name)),
                            args: vec![Operand::Copy(base_local)],
                        },
                        sp,
                    );
                }
            }
        } else {
            // Nested case: e.g. `buf.data[i] = val` or `a.b.arr[i] = val`.
            let root_op = self.lower_expr(current);
            let root_ty = self.lower_type_from_infer(&current.ty);
            let root_local = self.operand_to_local(root_op, &root_ty);

            // Load each intermediate into a mutable temp.
            let mut temps: Vec<LocalId> = Vec::new();
            let mut cur_local = root_local;
            for (field_name, field_ty) in &segments {
                let tmp = self.alloc_temp_mut(field_ty.clone());
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::FieldAccess {
                            base: Operand::Copy(cur_local),
                            field: field_name.clone(),
                        },
                    },
                    sp,
                );
                temps.push(tmp);
                cur_local = tmp;
            }

            // Index-assign on the deepest temp (the array).
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Index(cur_local, idx),
                    rvalue: Rvalue::Use(val),
                },
                sp,
            );

            // Write back bottom-up.
            for i in (0..segments.len()).rev() {
                let tmp = temps[i];
                let (field_name, _) = &segments[i];
                let parent = if i == 0 { root_local } else { temps[i - 1] };
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Field(parent, field_name.clone()),
                        rvalue: Rvalue::Use(Operand::Copy(tmp)),
                    },
                    sp,
                );
            }
        }

        Operand::Const(AirConst::Null)
    }

    fn lower_field_assign(
        &mut self,
        object: &TypedExpr,
        field: &str,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        let val = self.lower_expr(value);

        // Peel off Member layers to collect the path from root → immediate parent.
        // e.g. `a.b.c.field = val` → root=a, segments=[("b",ty_b),("c",ty_c)], field="field"
        let mut segments: Vec<(String, AirType)> = Vec::new();
        let mut current = object;
        loop {
            match &current.kind {
                TypedExprKind::Member { object: inner, member } => {
                    let field_ty = self.lower_type_from_infer(&current.ty);
                    segments.push((member.clone(), field_ty));
                    current = inner;
                }
                _ => break,
            }
        }
        segments.reverse(); // shallowest → deepest

        // Handle the root: Index needs read + write-back; everything else just finds the local.
        if let TypedExprKind::Index { object: arr_expr, index: idx_expr } = &current.kind {
            let arr_op = self.lower_expr(arr_expr);
            let arr_ty = self.lower_type_from_infer(&arr_expr.ty);
            let arr_local = self.operand_to_local(arr_op, &arr_ty);
            let idx_op = self.lower_expr(idx_expr);

            let elem_ty = self.lower_type_from_infer(&current.ty);
            let root_local = self.alloc_temp_mut(elem_ty);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(root_local),
                    rvalue: Rvalue::Index {
                        base: Operand::Copy(arr_local),
                        index: idx_op.clone(),
                    },
                },
                sp,
            );
            self.emit_field_chain(root_local, &segments, field, val, sp);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Index(arr_local, idx_op),
                    rvalue: Rvalue::Use(Operand::Copy(root_local)),
                },
                sp,
            );
        } else {
            let root_name = if let TypedExprKind::Identifier(name) = &current.kind {
                Some(name.clone())
            } else {
                None
            };
            let root_op = self.lower_expr(current);
            let root_ty = self.lower_type_from_infer(&current.ty);
            let root_local = self.operand_to_local(root_op, &root_ty);
            self.emit_field_chain(root_local, &segments, field, val, sp);
            // Write back to closure env if the root variable is a captured var,
            // or back to the global store if the root is a global variable.
            if let Some(ref name) = root_name {
                if let Some(env_id) = self.closure_env_param {
                    if self.closure_captures.contains(name) {
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Field(env_id, name.clone()),
                                rvalue: Rvalue::Use(Operand::Copy(root_local)),
                            },
                            sp,
                        );
                    }
                }
                if self.lookup_local(name).is_none()
                    && self.globals.iter().any(|g| g.name == *name)
                {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func: Callee::Named(format!("__aelys_global_set_{}", name)),
                            args: vec![Operand::Copy(root_local)],
                        },
                        sp,
                    );
                }
            }
        }

        Operand::Const(AirConst::Null)
    }

    /// Emit a read-modify-write chain for `root.seg0.seg1...segN.final_field = val`.
    ///
    /// `segments` are in order from shallowest (first field off root) to deepest.
    /// With zero segments this is a direct `Place::Field(root, final_field) = val`.
    fn emit_field_chain(
        &mut self,
        root_local: LocalId,
        segments: &[(String, AirType)],
        final_field: &str,
        val: Operand,
        sp: Option<Span>,
    ) {
        if segments.is_empty() {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(root_local, final_field.to_string()),
                    rvalue: Rvalue::Use(val),
                },
                sp,
            );
            return;
        }

        // Read each intermediate into a mutable temp.
        let mut temps: Vec<LocalId> = Vec::new();
        let mut current_local = root_local;
        for (field_name, field_ty) in segments {
            let tmp = self.alloc_temp_mut(field_ty.clone());
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(tmp),
                    rvalue: Rvalue::FieldAccess {
                        base: Operand::Copy(current_local),
                        field: field_name.clone(),
                    },
                },
                sp,
            );
            temps.push(tmp);
            current_local = tmp;
        }

        // Assign the value to the deepest temp's final field.
        self.emit(
            AirStmtKind::Assign {
                place: Place::Field(current_local, final_field.to_string()),
                rvalue: Rvalue::Use(val),
            },
            sp,
        );

        // Write back bottom-up: deepest → shallowest.
        for i in (0..segments.len()).rev() {
            let tmp = temps[i];
            let (field_name, _) = &segments[i];
            let parent = if i == 0 { root_local } else { temps[i - 1] };
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(parent, field_name.clone()),
                    rvalue: Rvalue::Use(Operand::Copy(tmp)),
                },
                sp,
            );
        }
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
        let is_void = Self::is_void_like(&result_ty);
        let result = if is_void { None } else { Some(self.alloc_temp_mut(result_ty)) };

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
        if let Some(result) = result {
            let then_val = self.lower_expr(then_branch);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(result),
                    rvalue: Rvalue::Use(then_val),
                },
                None,
            );
        } else {
            self.lower_expr_discard(then_branch);
        }
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(else_id);
        if let Some(result) = result {
            let else_val = self.lower_expr(else_branch);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(result),
                    rvalue: Rvalue::Use(else_val),
                },
                None,
            );
        } else {
            self.lower_expr_discard(else_branch);
        }
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(merge_id);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
    }

    // Lambda lowering: closures and function values in Aelys
    //
    // Every lambda, capturing or not, is lowered as a closure with an __env
    // parameter and wrapped in a fat pointer { fn_ptr, env_ptr }. Named functions
    // used as values also get wrapped in a fat pointer (with env_ptr = null).
    //
    // This uniformity is load-bearing: a call site receiving `fn(i64) -> i64`
    // cannot know whether it got a named function, a non-capturing lambda, or a
    // capturing closure. If these had different representations, indirect calls
    // would need two codepaths and the type system would need to track the
    // distinction. The alternative (generating thunks per named function, like
    // OCaml) was rejected for the same reason: more AIR, more generated code,
    // more surface for bugs.
    //
    // For capturing closures, the env struct is heap-allocated via Alloc (malloc).
    // Stack allocation would be unsound: closures can escape their creation scope
    // (returned from functions, stored in structs), and the env would dangle.
    // Escape analysis to decide stack vs heap is not implemented. The env is
    // intentionally leaked; the GC (@no_gc is the opt-out) will trace these
    // allocations once it exists. The representation won't need to change.
    //
    // Captures are by value at creation time. Mutating the original variable after
    // closure creation does not affect what the closure sees.
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
        // Always go through the closure path so every lambda gets an __env
        // parameter, ensuring a uniform calling convention for all function values.
        self.lower_function_as_closure(&fake_func);

        let sp = Some(self.span(&parent.span));
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let runtime_caps = self.runtime_captures(captures);

        if runtime_caps.is_empty() {
            // Non-capturing: fat pointer with null env
            self.emit_rvalue_to_temp(
                result_ty,
                Rvalue::ClosureCreate {
                    fn_name: lambda_name,
                    env: Operand::Const(AirConst::Null),
                },
                sp,
            )
        } else {
            // Capturing: heap-allocate env, store captures, build fat pointer
            let env_name = format!("__closure_env_{}", lambda_name);
            let env_ptr_ty = AirType::Ptr(Box::new(AirType::Struct(env_name.clone())));
            let env_ptr = self.alloc_temp(env_ptr_ty.clone());
            self.emit(
                AirStmtKind::Alloc {
                    local: env_ptr,
                    ty: AirType::Struct(env_name.clone()),
                },
                sp,
            );
            // Store each captured value into the env struct
            for (cap_name, _cap_ty) in &runtime_caps {
                let cap_val = if let Some(id) = self.lookup_local(cap_name) {
                    Operand::Copy(id)
                } else {
                    self.report_error(format!(
                        "ICE: captured variable `{}` not found in scope during closure lowering",
                        cap_name
                    ));
                    continue;
                };
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Field(env_ptr, cap_name.clone()),
                        rvalue: Rvalue::Use(cap_val),
                    },
                    sp,
                );
            }
            // Build fat pointer { fn_ptr, env_ptr }
            self.emit_rvalue_to_temp(
                result_ty,
                Rvalue::ClosureCreate {
                    fn_name: lambda_name,
                    env: Operand::Copy(env_ptr),
                },
                sp,
            )
        }
    }

    fn lower_match_expr(
        &mut self,
        scrutinee: &TypedExpr,
        arms: &[TypedMatchArm],
        parent: &TypedExpr,
    ) -> Operand {
        let sp = Some(self.span(&parent.span));
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let is_void = Self::is_void_like(&result_ty);
        // Only allocate a result local when the match produces a value.
        let result = if is_void { None } else { Some(self.alloc_temp_mut(result_ty)) };

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

            if let Some(result) = result {
                let arm_val = self.lower_expr(&arm.body);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(result),
                        rvalue: Rvalue::Use(arm_val),
                    },
                    None,
                );
            } else {
                self.lower_expr_discard(&arm.body);
            }
            self.seal_block(AirTerminator::Goto(merge_id));
        }

        // Lower the default block (wildcard or unreachable)
        self.fixup_block_id_noop(default_id);
        if let Some(wildcard) = wildcard_arm {
            if let Some(result) = result {
                let wc_val = self.lower_expr(&wildcard.body);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(result),
                        rvalue: Rvalue::Use(wc_val),
                    },
                    None,
                );
            } else {
                self.lower_expr_discard(&wildcard.body);
            }
            self.seal_block(AirTerminator::Goto(merge_id));
        } else {
            // Exhaustive match without wildcard -- all variants covered, default is unreachable
            self.seal_block(AirTerminator::Unreachable);
        }

        self.fixup_block_id_noop(merge_id);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
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
