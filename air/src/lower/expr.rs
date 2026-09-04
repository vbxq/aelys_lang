use super::place::PlaceMode;
use super::{LoweringContext, infer_to_int_size, lower_binop, lower_unop};
use crate::*;
use aelys_sema::{
    InferType, ResultAssertOnErr, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedMatchArm,
    TypedParam, TypedPattern, TypedStmt,
};

impl<'a> LoweringContext<'a> {
    fn is_void_like(ty: &AirType) -> bool {
        matches!(ty, AirType::Void | AirType::Opaque)
            || matches!(ty, AirType::Ptr(inner) if matches!(inner.as_ref(), AirType::Void))
    }

    // already owns its +1, so retaining it again would double-count
    fn emit_construction_field_retain(
        &mut self,
        field_air_ty: &AirType,
        value_op: &Operand,
        value_expr: &TypedExpr,
        sp: Option<Span>,
        what: &str,
    ) {
        let paths = match crate::rc_paths::rc_field_paths(field_air_ty, &self.structs, &self.enums)
        {
            crate::rc_paths::RcScan::None => return,
            crate::rc_paths::RcScan::Paths(paths) => paths,
            // skipping is only sound while generic structs never reach codegen; once they
            crate::rc_paths::RcScan::Undecidable(_) => return,
            crate::rc_paths::RcScan::RejectedMultiVariant(_) => return,
        };
        let mut prov = value_expr;
        while let TypedExprKind::Grouping(inner) = &prov.kind {
            prov = inner;
        }
        match &prov.kind {
            TypedExprKind::EnumVariant {
                enum_name, variant, ..
            } if enum_name == "Rc" && variant == "new" => {}
            TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => {}
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
                    if self.capture_slots.contains_key(&id) {
                        let value_ty = self.lower_type_from_infer(&expr.ty);
                        return self.emit_rvalue_to_temp(
                            value_ty,
                            Rvalue::Deref(Operand::Copy(id)),
                            sp,
                        );
                    }
                    if self.affine_category(&expr.ty).is_affine() {
                        Operand::Move(id)
                    } else {
                        Operand::Copy(id)
                    }
                } else if self.is_global_name(name) {
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&expr.ty),
                        Rvalue::Call {
                            func: Callee::Named(format!("__aelys_global_get_{}", name)),
                            args: Vec::new(),
                        },
                        sp,
                    )
                } else if matches!(expr.ty, InferType::Function { .. }) {
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
                let n = match &size.kind {
                    TypedExprKind::Int(v) => *v as u64,
                    _ => {
                        self.report_error(
                            "unsupported non-constant array size in AIR lowering: \
                             ArraySized requires a constant integer size expression"
                                .to_string(),
                        );
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
                self.lower_slice_expr(object, range, &expr.ty, sp)
            }

            // the reborrow short-circuit is gone: `&mut *p` is now just the deref line of
            TypedExprKind::Reference { operand, .. } => match self.place_addr(operand) {
                Some(addr) => Operand::Copy(addr.ptr),
                None => {
                    self.report_error(
                        "ICE: `&` of an expression that denotes no storage reached AIR lowering;                          sema must reject it (E0421)"
                            .to_string(),
                    );
                    Operand::Const(AirConst::Null)
                }
            },

            TypedExprKind::Deref(operand) => {
                let op = self.lower_expr(operand);
                let referent_air = self.lower_type_from_infer(&expr.ty);
                self.emit_rvalue_to_temp(referent_air, Rvalue::Deref(op), sp)
            }

            TypedExprKind::DerefAssign { target, value } => {
                let target_op = self.lower_expr(target);
                let target_ptr_ty = self.lower_type_from_infer(&target.ty);
                let t = self.operand_to_local(target_op, &target_ptr_ty);
                let v = self.lower_expr(value);
                let pointee_is_vec = matches!(value.ty, InferType::Vec(_));
                self.emit_vec_slot_acquire(pointee_is_vec, Some(&value.kind), &v, sp);
                if pointee_is_vec {
                    self.emit_cow_release_through_ptr(Operand::Copy(t), sp);
                }
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Deref(t),
                        rvalue: Rvalue::Use(v),
                    },
                    sp,
                );
                Operand::Const(AirConst::Null)
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
                if enum_name == "Rc" && variant == "new" {
                    return self.lower_rc_new(&expr.ty, args, sp);
                }
                // a read never touches the refcount
                if enum_name == "Rc" && variant == "get" {
                    return self.lower_rc_get(&expr.ty, args, sp);
                }
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
                if enum_name == "Vec" && variant == "try_as_unique_mut_slice" {
                    return self.lower_vec_try_as_unique_mut_slice(&expr.ty, args, sp);
                }
                if enum_name == "Vec" && variant == "len" {
                    return self.lower_vec_len(args, sp);
                }
                if enum_name == "Vec" && variant == "as_slice" {
                    return self.lower_vec_as_slice(&expr.ty, args, sp);
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
                self.emit_scope_affine_drops(scope_depth, tail.span);
                self.locals_by_name.truncate(scope_depth);
                result
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.lower_match_expr(scrutinee, arms, expr)
            }
            TypedExprKind::ResultAssert {
                scrutinee,
                ok_tag,
                payload_ty,
                on_err,
            } => self.lower_result_assert(expr, scrutinee, *ok_tag, payload_ty, on_err),
        }
    }

    pub(super) fn lower_expr_discard(&mut self, expr: &TypedExpr) {
        let sp = Some(self.span(&expr.span));
        match &expr.kind {
            TypedExprKind::Call { callee, args } => {
                let lowered_args: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let func = self.lower_callee(callee);
                let ret_ty = self.lower_type_from_infer(&expr.ty);
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

    fn lower_call_common(
        &mut self,
        func: Callee,
        args: Vec<Operand>,
        result_infer_ty: &InferType,
        sp: Option<Span>,
    ) -> Operand {
        let result_ty = self.lower_type_from_infer(result_infer_ty);
        // llvm. emit callvoid so codegen never tries to capture the result.
        if Self::is_void_like(&result_ty) {
            self.emit(AirStmtKind::CallVoid { func, args }, sp);
            Operand::Const(AirConst::Null)
        } else {
            self.emit_rvalue_to_temp(result_ty, Rvalue::Call { func, args }, sp)
        }
    }

    fn lower_assign_common(&mut self, name: &str, value: &TypedExpr, sp: Option<Span>) -> Operand {
        let val = self.lower_expr(value);
        if let Some(id) = self.lookup_local(name) {
            if let Some(key) = Self::air_drop_key(sp) {
                let old_drops = self.collect_affine_drops(key, |_| true);
                for (local, id_field) in old_drops {
                    self.emit_affine_drop(local, &id_field, sp);
                }
            }
            // previous buffer, so `v = v` (rc 1 -> 2 -> 1) never frees the buffer it keeps.
            let is_capture = self.capture_slots.contains_key(&id);
            let slot_is_vec = matches!(value.ty, InferType::Vec(_));
            self.emit_vec_slot_acquire(slot_is_vec, Some(&value.kind), &val, sp);
            if slot_is_vec {
                if is_capture {
                    self.emit_cow_release_through_ptr(Operand::Copy(id), sp);
                } else {
                    self.emit_cow_release(id, sp);
                }
            }
            let place = if is_capture {
                Place::Deref(id)
            } else {
                Place::Local(id)
            };
            self.emit(
                AirStmtKind::Assign {
                    place,
                    rvalue: Rvalue::Use(val.clone()),
                },
                sp,
            );
            if is_capture {
                return val;
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

    fn lower_slice_expr(
        &mut self,
        object: &TypedExpr,
        range: &TypedExpr,
        result_ty: &InferType,
        sp: Option<Span>,
    ) -> Operand {
        // `y[0]`, so building a zero-length view must not trap)
        let ptr_op = match self.projection_base(object) {
            Some(addr) => {
                // the detach repoints `v->ptr`, so a view built before it names the other owner's
                if matches!(result_ty, InferType::Slice { mutable: true, .. })
                    && Self::roots_a_vec(&object.ty)
                {
                    self.emit_cow_detach(addr.ptr, sp);
                }
                Operand::Copy(addr.ptr)
            }
            None => {
                self.report_error(
                    "ICE: slice of an expression that denotes no storage reached AIR lowering; \
                     sema must reject it (E0421)"
                        .to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
        let slice_air = self.lower_type_from_infer(result_ty);

        let (start, end) = match &range.kind {
            TypedExprKind::Range { start, end, .. } => (start.as_deref(), end.as_deref()),
            _ => (None, None),
        };
        if let Some(s) = start {
            if !matches!(&s.kind, TypedExprKind::Int(0)) {
                self.report_error(
                    "ICE: slice with a non-zero start bound reached AIR lowering; sema must \
                     reject it (E0425)"
                        .to_string(),
                );
            }
        }
        let len_op = match end {
            Some(e) => self.lower_expr(e),
            None => match &object.ty {
                InferType::Array(_, Some(n)) => {
                    Operand::Const(AirConst::Int(*n as i64, AirIntSize::I64))
                }
                InferType::Array(_, None) | InferType::Vec(_) | InferType::Slice { .. } => {
                    self.emit_rvalue_to_temp(AirType::I64, Rvalue::Len(ptr_op.clone()), sp)
                }
                _ => {
                    self.report_error(
                        "ICE: slice of a base with no derivable length reached AIR lowering; \
                         sema must reject it (E0425)"
                            .to_string(),
                    );
                    Operand::Const(AirConst::Int(0, AirIntSize::I64))
                }
            },
        };

        self.emit_rvalue_to_temp(
            slice_air,
            Rvalue::SliceFromParts {
                ptr: ptr_op,
                len: len_op,
            },
            sp,
        )
    }

    fn peel_grouping(mut expr: &TypedExpr) -> &TypedExpr {
        while let TypedExprKind::Grouping(inner) = &expr.kind {
            expr = inner;
        }
        expr
    }

    fn same_place(a: &TypedExpr, b: &TypedExpr) -> bool {
        let a = Self::peel_grouping(a);
        let b = Self::peel_grouping(b);
        match (&a.kind, &b.kind) {
            (TypedExprKind::Identifier(x), TypedExprKind::Identifier(y)) => x == y,
            (
                TypedExprKind::Member {
                    object: ao,
                    member: am,
                },
                TypedExprKind::Member {
                    object: bo,
                    member: bm,
                },
            ) => am == bm && Self::same_place(ao, bo),
            (
                TypedExprKind::Index {
                    object: ao,
                    index: ai,
                },
                TypedExprKind::Index {
                    object: bo,
                    index: bi,
                },
            ) => Self::same_place(ao, bo) && Self::same_operand(ai, bi),
            (TypedExprKind::Deref(ai), TypedExprKind::Deref(bi)) => Self::same_place(ai, bi),
            _ => false,
        }
    }

    fn same_operand(a: &TypedExpr, b: &TypedExpr) -> bool {
        let a = Self::peel_grouping(a);
        let b = Self::peel_grouping(b);
        if Self::same_place(a, b) {
            return true;
        }
        match (&a.kind, &b.kind) {
            (TypedExprKind::Int(x), TypedExprKind::Int(y)) => x == y,
            (TypedExprKind::Bool(x), TypedExprKind::Bool(y)) => x == y,
            (TypedExprKind::String(x), TypedExprKind::String(y)) => x == y,
            (TypedExprKind::Null, TypedExprKind::Null) => true,
            (
                TypedExprKind::Unary {
                    op: ao,
                    operand: aa,
                },
                TypedExprKind::Unary {
                    op: bo,
                    operand: bb,
                },
            ) => ao == bo && Self::same_operand(aa, bb),
            (
                TypedExprKind::Binary {
                    left: al,
                    op: ao,
                    right: ar,
                },
                TypedExprKind::Binary {
                    left: bl,
                    op: bo,
                    right: br,
                },
            ) => ao == bo && Self::same_operand(al, bl) && Self::same_operand(ar, br),
            (
                TypedExprKind::Cast {
                    expr: ae,
                    target: at,
                },
                TypedExprKind::Cast {
                    expr: be,
                    target: bt,
                },
            ) => at == bt && Self::same_operand(ae, be),
            (
                TypedExprKind::Call {
                    callee: ac,
                    args: aargs,
                },
                TypedExprKind::Call {
                    callee: bc,
                    args: bargs,
                },
            ) => {
                aargs.len() == bargs.len()
                    && Self::same_place(ac, bc)
                    && aargs
                        .iter()
                        .zip(bargs.iter())
                        .all(|(x, y)| Self::same_operand(x, y))
            }
            _ => false,
        }
    }

    fn lower_index_assign(
        &mut self,
        object: &TypedExpr,
        index: &TypedExpr,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        let compound_info = if let TypedExprKind::Binary { left, op, right } = &value.kind {
            match &Self::peel_grouping(left).kind {
                TypedExprKind::Index {
                    object: lo,
                    index: li,
                } if Self::same_place(lo, object) && Self::same_operand(li, index) => {
                    Some((*op, right.as_ref()))
                }
                _ => None,
            }
        } else {
            None
        };

        let idx = self.lower_expr(index);
        let val = if compound_info.is_none() {
            self.lower_expr(value)
        } else {
            Operand::Const(AirConst::Null)
        };

        let Some(base) = self.projection_base_mode(object, PlaceMode::Store) else {
            self.report_error(
                "ICE: indexed-assign target denotes no storage at AIR lowering; sema must \
                 reject it (E0421)"
                    .to_string(),
            );
            return Operand::Const(AirConst::Null);
        };

        let final_val = if let Some((op, rhs_expr)) = compound_info {
            let elem_ty = self.lower_type_from_infer(&value.ty);
            let current = self.emit_rvalue_to_temp(
                elem_ty.clone(),
                Rvalue::Index {
                    base: Operand::Copy(base.ptr),
                    index: idx.clone(),
                },
                sp,
            );
            let rhs = self.lower_expr(rhs_expr);
            let air_op = super::lower_binop(&op);
            self.emit_rvalue_to_temp(elem_ty, Rvalue::BinaryOp(air_op, current, rhs), sp)
        } else {
            val
        };

        if Self::roots_a_vec(&object.ty) {
            self.emit_cow_detach(base.ptr, sp);
        }
        self.emit(
            AirStmtKind::Assign {
                place: Place::Index(base.ptr, idx),
                rvalue: Rvalue::Use(final_val),
            },
            sp,
        );
        Operand::Const(AirConst::Null)
    }

    fn lower_field_assign(
        &mut self,
        object: &TypedExpr,
        field: &str,
        value: &TypedExpr,
        sp: Option<Span>,
    ) -> Operand {
        let compound_info = if let TypedExprKind::Binary { left, op, right } = &value.kind {
            match &Self::peel_grouping(left).kind {
                TypedExprKind::Member {
                    object: lo,
                    member: lm,
                } if lm == field && Self::same_place(lo, object) => Some((*op, right.as_ref())),
                _ => None,
            }
        } else {
            None
        };

        let val = if compound_info.is_none() {
            self.lower_expr(value)
        } else {
            Operand::Const(AirConst::Null)
        };

        let Some(base) = self.projection_base_mode(object, PlaceMode::Store) else {
            self.report_error(
                "ICE: field-assign target denotes no storage at AIR lowering; sema must \
                 reject it (E0421)"
                    .to_string(),
            );
            return Operand::Const(AirConst::Null);
        };

        let final_val = if let Some((op, rhs_expr)) = compound_info {
            let field_ty = self.lower_type_from_infer(&value.ty);
            let current = self.emit_rvalue_to_temp(
                field_ty.clone(),
                Rvalue::FieldAccess {
                    base: Operand::Copy(base.ptr),
                    field: field.to_string(),
                },
                sp,
            );
            let rhs = self.lower_expr(rhs_expr);
            let air_op = super::lower_binop(&op);
            self.emit_rvalue_to_temp(field_ty, Rvalue::BinaryOp(air_op, current, rhs), sp)
        } else {
            val
        };

        let mut obj = object;
        while let TypedExprKind::Grouping(inner) = &obj.kind {
            obj = inner;
        }
        let object_is_rc_handle = matches!(obj.ty, InferType::Rc(_));
        let lift_field_ty: Option<AirType> = if object_is_rc_handle {
            match &base.pointee {
                AirType::Struct(name) => match self.air_field_type_of(name, field) {
                    Some(fty @ AirType::Ptr(_)) => Some(fty),
                    _ => None,
                },
                _ => None,
            }
        } else {
            None
        };
        if let Some(field_ty) = lift_field_ty {
            let old = self.emit_rvalue_to_temp(
                field_ty,
                Rvalue::FieldAccess {
                    base: Operand::Copy(base.ptr),
                    field: field.to_string(),
                },
                sp,
            );
            self.emit_rc_release_operand(old, sp);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(base.ptr, field.to_string()),
                    rvalue: Rvalue::Use(final_val.clone()),
                },
                sp,
            );
            let prov_expr = match &compound_info {
                Some((_, rhs_expr)) => *rhs_expr,
                None => value,
            };
            let mut prov = prov_expr;
            while let TypedExprKind::Grouping(inner) = &prov.kind {
                prov = inner;
            }
            match &prov.kind {
                TypedExprKind::EnumVariant {
                    enum_name, variant, ..
                } if enum_name == "Rc" && variant == "new" => {}
                TypedExprKind::StructLiteral { .. } | TypedExprKind::EnumVariant { .. } => {}
                TypedExprKind::Identifier(_) | TypedExprKind::Member { .. } => {
                    self.emit_rc_retain(final_val, sp);
                }
                _ => {}
            }
        } else {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(base.ptr, field.to_string()),
                    rvalue: Rvalue::Use(final_val),
                },
                sp,
            );
        }

        Operand::Const(AirConst::Null)
    }

    fn lower_callee(&mut self, callee: &TypedExpr) -> Callee {
        match &callee.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(id) = self.lookup_local(name) {
                    if self.capture_slots.contains_key(&id) {
                        let op = self.lower_expr(callee);
                        let ty = self.lower_type_from_infer(&callee.ty);
                        return Callee::FnPtr(self.operand_to_local(op, &ty));
                    }
                    Callee::FnPtr(id)
                } else if self.is_global_name(name) {
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
        let result = if is_void {
            None
        } else {
            Some(self.alloc_temp_mut(result_ty))
        };

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

    // cannot know whether it got a named function, a non-capturing lambda, or a

    fn lower_vec_construct(
        &mut self,
        vec_ty: AirType,
        elem_ty: AirType,
        elements: Vec<Operand>,
        sp: Option<Span>,
    ) -> Operand {
        let count = elements.len() as i64;
        let vec_local = self.alloc_temp_mut(vec_ty.clone());
        let addr = self.addr_of_own_temp(vec_local, &vec_ty, sp);
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

    fn lower_vec_push(&mut self, args: &[TypedExpr], sp: Option<Span>) -> Operand {
        if args.len() != 2 {
            self.report_error(format!(
                "ICE: Vec::push expects 2 arguments, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let vec_addr = match self.projection_base(&args[0]) {
            Some(addr) => Operand::Copy(addr.ptr),
            None => {
                self.report_error(
                    "ICE: Vec::push target denotes no storage at AIR lowering; sema must \
                     reject it (E0421)"
                        .to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
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
        // p6, enumerated caller 2: the by-value copy is the intended abi; only the address
        let elem_addr = self.addr_of_own_temp(elem_slot, &elem_ty, sp);
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_vec_push".to_string()),
                args: vec![vec_addr, elem_addr],
            },
            sp,
        );
        Operand::Const(AirConst::Null)
    }

    fn lower_vec_len(&mut self, args: &[TypedExpr], sp: Option<Span>) -> Operand {
        if args.len() != 1 {
            self.report_error(format!(
                "ICE: Vec::len expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let Some(addr) = self.projection_base(&args[0]) else {
            self.report_error(
                "ICE: Vec::len target denotes no storage at AIR lowering".to_string(),
            );
            return Operand::Const(AirConst::Null);
        };
        self.emit_rvalue_to_temp(AirType::I64, Rvalue::Len(Operand::Copy(addr.ptr)), sp)
    }

    fn lower_vec_as_slice(
        &mut self,
        result_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        if args.len() != 1 {
            self.report_error(format!(
                "ICE: Vec::as_slice expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let Some(addr) = self.projection_base(&args[0]) else {
            self.report_error(
                "ICE: Vec::as_slice target denotes no storage at AIR lowering".to_string(),
            );
            return Operand::Const(AirConst::Null);
        };
        let len = self.emit_rvalue_to_temp(AirType::I64, Rvalue::Len(Operand::Copy(addr.ptr)), sp);
        self.emit_rvalue_to_temp(
            self.lower_type_from_infer(result_ty),
            Rvalue::SliceFromParts {
                ptr: Operand::Copy(addr.ptr),
                len,
            },
            sp,
        )
    }

    fn lower_vec_try_as_unique_mut_slice(
        &mut self,
        result_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        if args.len() != 1 {
            self.report_error(format!(
                "ICE: Vec::try_as_unique_mut_slice expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let vec_addr = match self.projection_base(&args[0]) {
            Some(addr) => Operand::Copy(addr.ptr),
            None => {
                self.report_error(
                    "ICE: Vec::try_as_unique_mut_slice target denotes no storage at AIR lowering"
                        .to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
        self.emit_rvalue_to_temp(
            self.lower_type_from_infer(result_ty),
            Rvalue::Call {
                func: Callee::Named("__aelys_vec_try_as_unique_mut_slice".to_string()),
                args: vec![vec_addr],
            },
            sp,
        )
    }

    fn lower_rc_new(&mut self, rc_ty: &InferType, args: &[TypedExpr], sp: Option<Span>) -> Operand {
        let inner_infer = match rc_ty {
            InferType::Rc(inner) => inner.as_ref(),
            _ => args.first().map(|a| &a.ty).unwrap_or(&InferType::Null),
        };
        let data_ty = self.lower_type_from_infer(inner_infer);
        let ptr_ty = AirType::Ptr(Box::new(data_ty.clone()));

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

    // no rcalloc, so a null is never tracked in the type table nor seen by the collector
    fn lower_rc_null(&mut self, rc_ty: &InferType) -> Operand {
        let inner_infer = match rc_ty {
            InferType::Rc(inner) => inner.as_ref(),
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
            declared_nogc: false,
            span: parent.span,
            captures: captures.to_vec(),
        };
        self.lower_function_as_closure(&fake_func);

        let sp = Some(self.span(&parent.span));
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let runtime_caps = self.runtime_captures(captures);

        // capturing an rc into a closure env is a use-after-free: the capture is a plain
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
            self.emit_rvalue_to_temp(
                result_ty,
                Rvalue::ClosureCreate {
                    fn_name: lambda_name,
                    env: Operand::Const(AirConst::Null),
                },
                sp,
            )
        } else {
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
            for (cap_name, cap_ty) in &runtime_caps {
                let cap_val = if let Some(id) = self.lookup_local(cap_name) {
                    if self.capture_slots.contains_key(&id) {
                        let value_ty = self.lower_type_from_infer(cap_ty);
                        self.emit_rvalue_to_temp(value_ty, Rvalue::Deref(Operand::Copy(id)), sp)
                    } else {
                        Operand::Copy(id)
                    }
                } else {
                    self.report_error(format!(
                        "ICE: captured variable `{}` not found in scope during closure lowering",
                        cap_name
                    ));
                    continue;
                };
                // takes a share here. there is no matching release: the env is deliberately leaked
                self.emit_vec_slot_acquire(matches!(cap_ty, InferType::Vec(_)), None, &cap_val, sp);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Field(env_ptr, cap_name.clone()),
                        rvalue: Rvalue::Use(cap_val),
                    },
                    sp,
                );
            }
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
        let result = if is_void {
            None
        } else {
            Some(self.alloc_temp_mut(result_ty))
        };

        let scrutinee_op = self.lower_expr(scrutinee);

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

        let tag_op = self.emit_rvalue_to_temp(
            AirType::I32,
            Rvalue::EnumTag {
                enum_name: enum_name.clone(),
                operand: scrutinee_op.clone(),
            },
            sp,
        );

        let merge_id = self.alloc_block_id();

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

        let default_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Switch {
            discr: tag_op,
            targets: switch_targets,
            default: default_id,
        });

        for (block_id, arm) in arm_blocks {
            self.fixup_block_id_noop(block_id);

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
            self.seal_block(AirTerminator::Unreachable);
        }

        self.fixup_block_id_noop(merge_id);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
    }

    fn lower_result_assert(
        &mut self,
        node: &TypedExpr,
        scrutinee: &TypedExpr,
        ok_tag: u32,
        payload_ty: &InferType,
        on_err: &ResultAssertOnErr,
    ) -> Operand {
        let sp = Some(self.span(&node.span));
        let result_ty = self.lower_type_from_infer(payload_ty);
        let is_void = Self::is_void_like(&result_ty);
        let result = if is_void {
            None
        } else {
            Some(self.alloc_temp_mut(result_ty))
        };

        let scrutinee_op = self.lower_expr(scrutinee);
        let enum_name = match &scrutinee.ty {
            InferType::Enum(name, _) => name.clone(),
            _ => {
                self.report_error(format!(
                    "result assert scrutinee is not an enum type: {:?}",
                    scrutinee.ty
                ));
                return Operand::Const(AirConst::Null);
            }
        };

        let tag_op = self.emit_rvalue_to_temp(
            AirType::I32,
            Rvalue::EnumTag {
                enum_name: enum_name.clone(),
                operand: scrutinee_op.clone(),
            },
            sp,
        );

        let ok_block = self.alloc_block_id();
        let err_block = self.alloc_block_id();
        let merge = self.alloc_block_id();

        self.seal_block(AirTerminator::Switch {
            discr: tag_op,
            targets: vec![(AirConst::Int(ok_tag as i64, AirIntSize::I32), ok_block)],
            default: err_block,
        });

        self.fixup_block_id_noop(ok_block);
        if let Some(result) = result {
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(result),
                    rvalue: Rvalue::EnumPayload {
                        enum_name,
                        tag: ok_tag,
                        operand: scrutinee_op,
                        field_index: 0,
                    },
                },
                sp,
            );
        }
        self.seal_block(AirTerminator::Goto(merge));

        self.fixup_block_id_noop(err_block);
        self.seal_block(match on_err {
            ResultAssertOnErr::Panic(msg) => AirTerminator::Panic {
                message: msg.clone(),
                span: sp,
            },
            ResultAssertOnErr::Unreachable => AirTerminator::Unreachable,
        });

        self.fixup_block_id_noop(merge);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
    }

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
