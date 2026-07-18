use super::{LoweringContext, infer_to_int_size, lower_binop, lower_unop};
use crate::*;
use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedMatchArm, TypedParam,
    TypedPattern, TypedStmt,
};

impl<'a> LoweringContext<'a> {
    fn is_void_like(ty: &AirType) -> bool {
        matches!(ty, AirType::Void | AirType::Opaque)
            || matches!(ty, AirType::Ptr(inner) if matches!(inner.as_ref(), AirType::Void))
    }

    // a carrier field is retained only when it clones a live reference; a fresh literal
    // already owns its +1, so retaining it again would double-count
    fn emit_construction_field_retain(
        &mut self,
        field_air_ty: &AirType,
        value_op: &Operand,
        value_expr: &TypedExpr,
        sp: Option<Span>,
        what: &str,
    ) {
        let paths = match crate::rc_paths::rc_field_paths(field_air_ty, &self.structs, &self.enums) {
            crate::rc_paths::RcScan::None => return,
            crate::rc_paths::RcScan::Paths(paths) => paths,
            // skipping is only sound while generic structs never reach codegen; once they
            // monomorphize, an Rc carrier will slip through here and leak or UAF
            crate::rc_paths::RcScan::Undecidable(_) => return,
            // already rejected at the carrier's `let`, so emit nothing here
            crate::rc_paths::RcScan::RejectedMultiVariant(_) => return,
        };
        let mut prov = value_expr;
        while let TypedExprKind::Grouping(inner) = &prov.kind {
            prov = inner;
        }
        match &prov.kind {
            // fresh: Rc::new already set refcount to 1
            TypedExprKind::EnumVariant { enum_name, variant, .. }
                if enum_name == "Rc" && variant == "new" => {}
            TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => {}
            // a clone of a live reference, so retain every transitive leaf
            TypedExprKind::Identifier(_) | TypedExprKind::Member { .. } => {
                self.emit_carrier_field_retains(value_op.clone(), field_air_ty, &paths, sp);
            }
            TypedExprKind::Call { .. } => {
                self.report_error(format!(
                    "[rc-stage3a] {what} is initialized from a call returning an `Rc<T>`-bearing \
                     value; ownership transfer into a carrier field is not supported yet"
                ));
            }
            _ => {
                self.report_error(format!(
                    "[rc-stage3a] {what} is initialized from a conditional/compound expression \
                     producing an `Rc<T>`-bearing value; only a direct reference (clone) or a \
                     fresh literal is supported yet"
                ));
            }
        }
    }

    fn air_field_type_of(&self, struct_name: &str, field: &str) -> Option<AirType> {
        self.structs
            .iter()
            .find(|s| s.name == struct_name)
            .and_then(|d| d.fields.iter().find(|f| f.name == field))
            .map(|f| f.ty.clone())
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
                let mut lowered_fields: Vec<(String, Operand)> = Vec::with_capacity(fields.len());
                for (fname, fval) in fields {
                    let op = self.lower_expr(fval);
                    if let Some(field_air_ty) = self.air_field_type_of(name, fname) {
                        self.emit_construction_field_retain(
                            &field_air_ty,
                            &op,
                            fval,
                            sp,
                            &format!("field `{name}.{fname}`"),
                        );
                    }
                    lowered_fields.push((fname.clone(), op));
                }
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
                let vec_ty = self.lower_type_from_infer(&expr.ty);
                let elem_ty = match &vec_ty {
                    AirType::Vec(inner) => (**inner).clone(),
                    other => {
                        self.report_error(format!(
                            "ICE: vec literal has non-Vec AIR type `{other:?}` at lowering"
                        ));
                        AirType::I64
                    }
                };
                self.lower_vec_construct(vec_ty, elem_ty, lowered, sp)
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
                // sema carries the Rc and Vec intrinsics as EnumVariant, so intercept
                // them here before the generic enum path
                // no retain: the alloc already sets refcount to 1
                if enum_name == "Rc" && variant == "new" {
                    return self.lower_rc_new(&expr.ty, args, sp);
                }
                // a read never touches the refcount
                if enum_name == "Rc" && variant == "get" {
                    return self.lower_rc_get(&expr.ty, args, sp);
                }
                // a null data pointer for cycle construction, not an allocation
                if enum_name == "Rc" && variant == "null" {
                    return self.lower_rc_null(&expr.ty);
                }
                if enum_name == "Vec" && variant == "new" {
                    let vec_ty = self.lower_type_from_infer(&expr.ty);
                    let elem_ty = match &vec_ty {
                        AirType::Vec(inner) => (**inner).clone(),
                        other => {
                            self.report_error(format!(
                                "ICE: Vec::new has non-Vec AIR type `{other:?}` at lowering"
                            ));
                            AirType::I64
                        }
                    };
                    return self.lower_vec_construct(vec_ty, elem_ty, Vec::new(), sp);
                }
                if enum_name == "Vec" && variant == "push" {
                    return self.lower_vec_push(args, sp);
                }
                let mut payload: Vec<Operand> = Vec::with_capacity(args.len());
                for arg in args {
                    let op = self.lower_expr(arg);
                    let payload_air_ty = self.lower_type_from_infer(&arg.ty);
                    self.emit_construction_field_retain(
                        &payload_air_ty,
                        &op,
                        arg,
                        sp,
                        &format!("payload of `{enum_name}::{variant}`"),
                    );
                    payload.push(op);
                }
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

        // Detect compound index assignment pattern from parser desugaring:
        //   arr[idx] += rhs  →  arr[idx] = arr[idx] + rhs
        // The parser clones index/object expressions, so they'd be re-evaluated
        // on the RHS (wrong if they have side effects).  Defer value computation
        // to after the path is established so we can reuse the path locals.
        let compound_info = if let TypedExprKind::Binary { left, op, right } = &value.kind {
            if let TypedExprKind::Index { .. } = &left.kind {
                Some((*op, right.as_ref()))
            } else {
                None
            }
        } else {
            None
        };

        let val = if compound_info.is_none() {
            self.lower_expr(value)
        } else {
            Operand::Const(AirConst::Null) // placeholder, computed below
        };

        // Collect the full access path (mix of Member and Index layers) from
        // `object` back to the root variable.  Handles all patterns:
        //   arr[i] = val, arr[i][j] = val, s.arr[i] = val,
        //   s.arr[i][j] = val, rows[i].cells[j] = val, etc.
        enum PathStep<'a> {
            Field { name: String, result_ty: AirType },
            Index { idx_expr: &'a TypedExpr, result_ty: AirType },
        }

        let mut steps: Vec<PathStep<'_>> = Vec::new();
        let mut walk = object;
        loop {
            match &walk.kind {
                TypedExprKind::Index { object, index: nested_idx } => {
                    let result_ty = self.lower_type_from_infer(&walk.ty);
                    steps.push(PathStep::Index { idx_expr: nested_idx, result_ty });
                    walk = object;
                }
                TypedExprKind::Member { object, member } => {
                    let result_ty = self.lower_type_from_infer(&walk.ty);
                    steps.push(PathStep::Field { name: member.clone(), result_ty });
                    walk = object;
                }
                _ => break,
            }
        }
        steps.reverse(); // root → outermost

        // `walk` is now the root expression (usually an Identifier).
        let root_name = if let TypedExprKind::Identifier(name) = &walk.kind {
            Some(name.clone())
        } else {
            None
        };
        let root_op = self.lower_expr(walk);
        let root_ty = self.lower_type_from_infer(&walk.ty);
        let root_local = self.operand_to_local(root_op, &root_ty);

        // Read phase: load each intermediate into a mutable temp.
        enum WriteBack {
            Field(String),
            Index(Operand),
        }
        struct TempInfo {
            local: LocalId,
            parent: LocalId,
            wb: WriteBack,
        }

        let mut temps: Vec<TempInfo> = Vec::new();
        let mut cur_local = root_local;

        for step in &steps {
            match step {
                PathStep::Field { name, result_ty } => {
                    let tmp = self.alloc_temp_mut(result_ty.clone());
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::FieldAccess {
                                base: Operand::Copy(cur_local),
                                field: name.clone(),
                            },
                        },
                        sp,
                    );
                    temps.push(TempInfo { local: tmp, parent: cur_local, wb: WriteBack::Field(name.clone()) });
                    cur_local = tmp;
                }
                PathStep::Index { idx_expr, result_ty } => {
                    let index_op = self.lower_expr(idx_expr);
                    let tmp = self.alloc_temp_mut(result_ty.clone());
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::Index {
                                base: Operand::Copy(cur_local),
                                index: index_op.clone(),
                            },
                        },
                        sp,
                    );
                    temps.push(TempInfo { local: tmp, parent: cur_local, wb: WriteBack::Index(index_op) });
                    cur_local = tmp;
                }
            }
        }

        // For compound index assigns, compute the value now using the path-loaded
        // cur_local instead of re-evaluating the object path.
        let final_val = if let Some((op, rhs_expr)) = compound_info {
            let elem_ty = self.lower_type_from_infer(&value.ty);
            let current = self.emit_rvalue_to_temp(
                elem_ty.clone(),
                Rvalue::Index {
                    base: Operand::Copy(cur_local),
                    index: idx.clone(),
                },
                sp,
            );
            let rhs = self.lower_expr(rhs_expr);
            let air_op = super::lower_binop(&op);
            self.emit_rvalue_to_temp(
                elem_ty,
                Rvalue::BinaryOp(air_op, current, rhs),
                sp,
            )
        } else {
            val
        };

        // Write the value at the innermost level.
        self.emit(
            AirStmtKind::Assign {
                place: Place::Index(cur_local, idx),
                rvalue: Rvalue::Use(final_val),
            },
            sp,
        );

        // Write-back phase: propagate modifications back up to the root.
        for info in temps.iter().rev() {
            match &info.wb {
                WriteBack::Field(name) => {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Field(info.parent, name.clone()),
                            rvalue: Rvalue::Use(Operand::Copy(info.local)),
                        },
                        sp,
                    );
                }
                WriteBack::Index(index_op) => {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Index(info.parent, index_op.clone()),
                            rvalue: Rvalue::Use(Operand::Copy(info.local)),
                        },
                        sp,
                    );
                }
            }
        }

        // Closure env / global write-back for root.
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

        Operand::Const(AirConst::Null)
    }

    fn lower_field_assign(
        &mut self,
        object: &TypedExpr,
        field: &str,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        // Detect compound field assignment from parser desugaring:
        //   obj.field += rhs  →  obj.field = obj.field + rhs
        // When the object path contains side-effecting expressions (e.g.
        // arr[f()].field += rhs), the desugared form evaluates them twice.
        // Detect the pattern and defer value computation to after the path
        // is established, so we can reuse the already-loaded current value.
        let compound_info = if let TypedExprKind::Binary { left, op, right } = &value.kind {
            if let TypedExprKind::Member { .. } = &left.kind {
                Some((*op, right.as_ref()))
            } else {
                None
            }
        } else {
            None
        };

        // For non-compound assigns, lower the value normally.
        // For compound assigns, we'll compute val later using the path locals.
        let val = if compound_info.is_none() {
            self.lower_expr(value)
        } else {
            // Placeholder — will be replaced below
            Operand::Const(AirConst::Null)
        };

        // Collect the full access path from root → immediate parent of the
        // assigned field.  Each step is either a `.field` or `[idx]` access.
        // This handles arbitrary mixes like `a.b[i].c.d[j].field = val`.
        enum PathStep<'a> {
            Field { name: String, result_ty: AirType },
            Index { idx_expr: &'a TypedExpr, result_ty: AirType },
        }

        let mut steps: Vec<PathStep<'_>> = Vec::new();
        let mut current = object;
        loop {
            match &current.kind {
                TypedExprKind::Member { object: inner, member } => {
                    let result_ty = self.lower_type_from_infer(&current.ty);
                    steps.push(PathStep::Field { name: member.clone(), result_ty });
                    current = inner;
                }
                TypedExprKind::Index { object: inner, index: idx_expr } => {
                    let result_ty = self.lower_type_from_infer(&current.ty);
                    steps.push(PathStep::Index { idx_expr, result_ty });
                    current = inner;
                }
                _ => break,
            }
        }
        steps.reverse(); // root → deepest

        // `current` is now the root expression (usually an Identifier).
        let root_name = if let TypedExprKind::Identifier(name) = &current.kind {
            Some(name.clone())
        } else {
            None
        };
        let root_op = self.lower_expr(current);
        let root_ty = self.lower_type_from_infer(&current.ty);
        let root_local = self.operand_to_local(root_op, &root_ty);

        // Read phase: load each intermediate into a mutable temp.
        enum WriteBack {
            Field(String),
            Index(Operand),
        }
        struct TempInfo {
            local: LocalId,
            parent: LocalId,
            wb: WriteBack,
        }

        let mut temps: Vec<TempInfo> = Vec::new();
        let mut cur_local = root_local;

        for step in &steps {
            match step {
                PathStep::Field { name, result_ty } => {
                    let tmp = self.alloc_temp_mut(result_ty.clone());
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::FieldAccess {
                                base: Operand::Copy(cur_local),
                                field: name.clone(),
                            },
                        },
                        sp,
                    );
                    temps.push(TempInfo { local: tmp, parent: cur_local, wb: WriteBack::Field(name.clone()) });
                    cur_local = tmp;
                }
                PathStep::Index { idx_expr, result_ty } => {
                    let idx_op = self.lower_expr(idx_expr);
                    let tmp = self.alloc_temp_mut(result_ty.clone());
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::Index {
                                base: Operand::Copy(cur_local),
                                index: idx_op.clone(),
                            },
                        },
                        sp,
                    );
                    temps.push(TempInfo { local: tmp, parent: cur_local, wb: WriteBack::Index(idx_op) });
                    cur_local = tmp;
                }
            }
        }

        // For compound field assigns, compute the value now using the path-loaded
        // intermediates instead of the pre-lowered val (which would re-evaluate
        // side-effecting path expressions).
        let final_val = if let Some((op, rhs_expr)) = compound_info {
            let field_ty = self.lower_type_from_infer(&value.ty);
            let current = self.emit_rvalue_to_temp(
                field_ty.clone(),
                Rvalue::FieldAccess {
                    base: Operand::Copy(cur_local),
                    field: field.to_string(),
                },
                sp,
            );
            let rhs = self.lower_expr(rhs_expr);
            let air_op = super::lower_binop(&op);
            self.emit_rvalue_to_temp(
                field_ty,
                Rvalue::BinaryOp(air_op, current, rhs),
                sp,
            )
        } else {
            val
        };

        // reassigning an Rc field through an Rc handle must stay balanced, so the store is
        // lifted into: load old, release old, store new, retain new. skipping the release
        // orphans the old reference, skipping the retain under-counts the new one
        let lift_field_ty: Option<AirType> = match self.local_air_type(cur_local) {
            Some(AirType::Ptr(inner)) => match inner.as_ref() {
                AirType::Struct(name) => match self.air_field_type_of(name, field) {
                    Some(fty @ AirType::Ptr(_)) => Some(fty),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        if let Some(field_ty) = lift_field_ty {
            let old = self.emit_rvalue_to_temp(
                field_ty,
                Rvalue::FieldAccess {
                    base: Operand::Copy(cur_local),
                    field: field.to_string(),
                },
                sp,
            );
            self.emit_rc_release_operand(old, sp);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(cur_local, field.to_string()),
                    rvalue: Rvalue::Use(final_val.clone()),
                },
                sp,
            );
            // the final retain is gated on provenance, like the construction site above:
            // retaining a fresh value would orphan the +1 it already carries
            let prov_expr = match &compound_info {
                Some((_, rhs_expr)) => *rhs_expr,
                None => value,
            };
            let mut prov = prov_expr;
            while let TypedExprKind::Grouping(inner) = &prov.kind {
                prov = inner;
            }
            match &prov.kind {
                // fresh, so it already holds a +1 for this slot
                TypedExprKind::EnumVariant { enum_name, variant, .. }
                    if enum_name == "Rc" && variant == "new" => {}
                TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => {}
                // shared, so the source keeps its count and this slot needs its own
                TypedExprKind::Identifier(_) | TypedExprKind::Member { .. } => {
                    self.emit_rc_retain(final_val, sp);
                }
                // anything else is treated as owned, the slot takes over its count
                _ => {}
            }
        } else {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(cur_local, field.to_string()),
                    rvalue: Rvalue::Use(final_val),
                },
                sp,
            );
        }

        // Write-back phase: propagate modifications back up to the root.
        for info in temps.iter().rev() {
            match &info.wb {
                WriteBack::Field(name) => {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Field(info.parent, name.clone()),
                            rvalue: Rvalue::Use(Operand::Copy(info.local)),
                        },
                        sp,
                    );
                }
                WriteBack::Index(idx_op) => {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Index(info.parent, idx_op.clone()),
                            rvalue: Rvalue::Use(Operand::Copy(info.local)),
                        },
                        sp,
                    );
                }
            }
        }

        // Write back to closure env or global store if needed.
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

    // codegen injects the element size from the address operand's pointee type
    fn lower_vec_construct(
        &mut self,
        vec_ty: AirType,
        elem_ty: AirType,
        elements: Vec<Operand>,
        sp: Option<Span>,
    ) -> Operand {
        let count = elements.len() as i64;
        // must be mutable: push writes ptr/len/cap later, and the address needs a real alloca
        let vec_local = self.alloc_temp_mut(vec_ty.clone());
        let addr = self.emit_rvalue_to_temp(
            AirType::Ptr(Box::new(vec_ty)),
            Rvalue::AddressOf(vec_local),
            sp,
        );
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_vec_init".to_string()),
                args: vec![addr, Operand::Const(AirConst::IntLiteral(count))],
            },
            sp,
        );
        for (i, elem_op) in elements.into_iter().enumerate() {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Index(vec_local, Operand::Const(AirConst::IntLiteral(i as i64))),
                    rvalue: Rvalue::Use(elem_op),
                },
                sp,
            );
        }
        let _ = elem_ty; // element type is recovered by codegen from the Vec local
        Operand::Copy(vec_local)
    }

    // the element is spilled to a mutable temp only so its address can be taken
    fn lower_vec_push(&mut self, args: &[TypedExpr], sp: Option<Span>) -> Operand {
        if args.len() != 2 {
            self.report_error(format!(
                "ICE: Vec::push expects 2 arguments, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        // The Vec l-value: lower it to the local holding the fat-ptr, take &v.
        let vec_ty = self.lower_type_from_infer(&args[0].ty);
        let vec_op = self.lower_expr(&args[0]);
        let vec_local = match vec_op {
            Operand::Copy(id) | Operand::Move(id) => id,
            _ => {
                self.report_error(
                    "Vec::push target must be a Vec variable (an addressable l-value)".to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
        let vec_addr = self.emit_rvalue_to_temp(
            AirType::Ptr(Box::new(vec_ty)),
            Rvalue::AddressOf(vec_local),
            sp,
        );
        let elem_ty = self.lower_type_from_infer(&args[1].ty);
        let elem_op = self.lower_expr(&args[1]);
        let elem_slot = self.alloc_temp_mut(elem_ty.clone());
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(elem_slot),
                rvalue: Rvalue::Use(elem_op),
            },
            sp,
        );
        let elem_addr = self.emit_rvalue_to_temp(
            AirType::Ptr(Box::new(elem_ty)),
            Rvalue::AddressOf(elem_slot),
            sp,
        );
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_vec_push".to_string()),
                args: vec![vec_addr, elem_addr],
            },
            sp,
        );
        Operand::Const(AirConst::Null)
    }

    fn lower_rc_new(
        &mut self,
        rc_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        let inner_infer = match rc_ty {
            InferType::Rc(inner) => inner.as_ref(),
            // sema guarantees Rc<_>, this fallback just keeps the AIR well-typed
            _ => args.first().map(|a| &a.ty).unwrap_or(&InferType::Null),
        };
        let data_ty = self.lower_type_from_infer(inner_infer);
        let ptr_ty = AirType::Ptr(Box::new(data_ty.clone()));

        // evaluate the payload before the alloc local exists
        let data_op = args
            .first()
            .map(|a| self.lower_expr(a))
            .unwrap_or(Operand::Const(AirConst::ZeroInit(data_ty.clone())));

        let ptr_local = self.alloc_temp(ptr_ty);
        self.emit(
            AirStmtKind::RcAlloc {
                local: ptr_local,
                ty: data_ty,
            },
            sp,
        );
        self.emit(
            AirStmtKind::Assign {
                place: Place::Deref(ptr_local),
                rvalue: Rvalue::Use(data_op),
            },
            sp,
        );
        Operand::Copy(ptr_local)
    }

    // the value is copied out before any scope-exit release, so the read stays sound
    fn lower_rc_get(
        &mut self,
        result_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        let inner_ty = self.lower_type_from_infer(result_ty);
        let handle = args
            .first()
            .map(|a| self.lower_expr(a))
            .unwrap_or(Operand::Const(AirConst::Null));
        self.emit_rvalue_to_temp(inner_ty, Rvalue::Deref(handle), sp)
    }

    // no RcAlloc, so a null is never tracked in the type table nor seen by the collector
    fn lower_rc_null(&mut self, rc_ty: &InferType) -> Operand {
        let inner_infer = match rc_ty {
            InferType::Rc(inner) => inner.as_ref(),
            // sema guarantees Rc<_>, this fallback just keeps the AIR well-typed
            _ => &InferType::Null,
        };
        let data_ty = self.lower_type_from_infer(inner_infer);
        let ptr_ty = AirType::Ptr(Box::new(data_ty));
        self.emit_rvalue_to_temp(ptr_ty, Rvalue::Use(Operand::Const(AirConst::Null)), None)
    }

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

        // capturing an Rc into a closure env is a use-after-free: the capture is a plain
        // copy with no retain, yet the outer Rc is still released at scope exit
        for (cap_name, cap_ty) in &runtime_caps {
            let cap_air = self.lower_type_from_infer(cap_ty);
            // only a resolved carrier rejects; an undecidable generic must not
            let carrier_capture = matches!(
                crate::rc_paths::rc_field_paths(&cap_air, &self.structs, &self.enums),
                crate::rc_paths::RcScan::Paths(_)
            );
            if cap_ty.is_rc() || cap_ty.contains_rc() || carrier_capture {
                self.report_error(format!(
                    "[rc-stage1] closure captures `{cap_name}` of type `{cap_ty}` which is or contains \
                     an `Rc<T>`; capturing an Rc (or a carrier of one) in a closure is not supported"
                ));
            }
        }

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
