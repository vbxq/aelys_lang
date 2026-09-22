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
        match aelys_sema::rc_init::rc_init_provenance(value_expr) {
            aelys_sema::rc_init::RcInit::Fresh => {}
            aelys_sema::rc_init::RcInit::Borrowed => {
                self.emit_carrier_field_retains(value_op.clone(), field_air_ty, &paths, sp);
            }
            aelys_sema::rc_init::RcInit::Unaccountable(blocker) => {
                self.report_unsupported(aelys_sema::rc_init::rc_init_refusal(blocker, what));
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
            TypedExprKind::Char(cp) => Operand::Const(AirConst::Char(*cp)),
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
                let mut pair = self
                    .lower_pinned_operands(&[left.as_ref(), right.as_ref()], sp)
                    .into_iter();
                let (Some(l), Some(r)) = (pair.next(), pair.next()) else {
                    self.report_ice(
                        "ICE: a binary operation lowered fewer than two operands".to_string(),
                    );
                    return Operand::Const(AirConst::Null);
                };
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
                let func = self.lower_callee(callee, args);
                let lowered_args = self.lower_call_args(callee, args, sp);
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
                if member == "len"
                    && matches!(
                        object.ty,
                        InferType::Slice { .. } | InferType::Vec(_) | InferType::Array(..)
                    )
                {
                    return self.lower_len_member(object, sp);
                }
                let base = self.lower_expr(object);
                self.pose_fresh_temp(&base);
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
                let values: Vec<&TypedExpr> = fields.iter().map(|(_, e)| e.as_ref()).collect();
                for (i, (fname, fval)) in fields.iter().enumerate() {
                    let op = self.lower_expr(fval);
                    let op = self.hold_operand(op, fval, &values[i + 1..], false, sp);
                    let op = self.own_str_for_store(op, &fval.ty, sp);
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
                let refs: Vec<&TypedExpr> = elements.iter().collect();
                let lowered = self.lower_pinned_operands_owning(&refs, true, sp);
                let n = lowered.len() as u64;
                let elem_ty = match &expr.ty {
                    InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                    other => {
                        self.report_ice(format!(
                            "ICE: array literal has non-array type `{}` at AIR lowering",
                            other
                        ));
                        AirType::I64
                    }
                };
                let arr_ty = AirType::Array(Box::new(elem_ty), n);
                let arr_local = self.alloc_temp_mut(arr_ty);
                if self.counted(&expr.ty) {
                    self.fresh_str_results.insert(arr_local);
                }
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
                // the count comes from the type, never from the size expression: the optimizer
                let (elem_ty, n) = match &expr.ty {
                    InferType::Array(inner, len) => {
                        let elem_ty = self.lower_type_from_infer(inner);
                        let len = if crate::ablation::array_length_from_size_expr() {
                            match &size.kind {
                                aelys_sema::TypedExprKind::Int(v) => Some(*v as u64),
                                _ => None,
                            }
                        } else {
                            *len
                        };
                        match len {
                            Some(n) => (elem_ty, n),
                            None => {
                                self.report_unsupported(non_constant_array_size_message(size));
                                (elem_ty, 0)
                            }
                        }
                    }
                    other => {
                        self.report_ice(format!(
                            "ICE: ArraySized has non-array type `{}` at AIR lowering",
                            other
                        ));
                        (AirType::I64, 0)
                    }
                };
                self.check_stack_array_size(&elem_ty, n);
                let arr_ty = AirType::Array(Box::new(elem_ty.clone()), n);
                let arr_local = self.alloc_temp_mut(arr_ty);
                if self.counted(&expr.ty) {
                    self.fresh_str_results.insert(arr_local);
                }
                let fill_op = if let Some(fv) = fill_value {
                    let op = self.lower_expr(fv);
                    if n > 0 {
                        self.own_str_for_store(op, &fv.ty, sp)
                    } else {
                        op
                    }
                } else {
                    Operand::Const(AirConst::ZeroInit(elem_ty))
                };
                for i in 0..n {
                    if i > 0 {
                        self.retain_str_fill(&fill_op, sp);
                    }
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
                let refs: Vec<&TypedExpr> = elements.iter().collect();
                let lowered = self.lower_pinned_operands_owning(&refs, true, sp);
                let vec_ty = self.lower_type_from_infer(&expr.ty);
                let elem_ty = match &vec_ty {
                    AirType::Vec(inner) => (**inner).clone(),
                    other => {
                        self.report_ice(format!(
                            "ICE: vec literal has non-Vec AIR type `{other:?}` at lowering"
                        ));
                        AirType::I64
                    }
                };
                self.lower_vec_construct(vec_ty, elem_ty, lowered, sp)
            }

            TypedExprKind::Index { object, index } => {
                let obj = if let TypedExprKind::Member {
                    object: text,
                    member,
                } = &super::pin::peel(object).kind
                    && member == "bytes"
                    && matches!(text.ty, InferType::String)
                    && super::pin::may_free(index)
                {
                    let text_op = self.lower_expr(text);
                    let pinned = self.pin_str_read(text_op, sp);
                    self.emit_rvalue_to_temp(
                        self.lower_type_from_infer(&object.ty),
                        Rvalue::FieldAccess {
                            base: pinned,
                            field: member.clone(),
                        },
                        sp,
                    )
                } else {
                    self.lower_expr(object)
                };
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
                    self.report_ice(
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
                let v = self.own_str_for_store(v, &value.ty, sp);
                let pointee_is_vec = matches!(value.ty, InferType::Vec(_));
                if !(pointee_is_vec && self.claim_owned_str_temp(&v)) {
                    self.emit_vec_slot_acquire(pointee_is_vec, Some(&value.kind), &v, sp);
                }
                if pointee_is_vec {
                    self.emit_cow_release_through_ptr(Operand::Copy(t), sp);
                }
                let through_mut = matches!(target.ty, InferType::Ref { mutable: true, .. });
                let old_pointee = (through_mut && self.counted(&value.ty)).then(|| {
                    let pointee_ty = self.lower_type_from_infer(&value.ty);
                    self.emit_rvalue_to_temp(pointee_ty, Rvalue::Deref(Operand::Copy(t)), sp)
                });
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Deref(t),
                        rvalue: Rvalue::Use(v),
                    },
                    sp,
                );
                if let Some(Operand::Copy(old) | Operand::Move(old)) = old_pointee {
                    self.emit_str_release(old, sp);
                }
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
                            self.report_ice(format!(
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
                if enum_name == "Vec" && variant == "pop" {
                    return self.lower_vec_pop(&expr.ty, args, sp);
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
                if enum_name == "string" && variant == "from_char" {
                    let lowered: Vec<Operand> =
                        args.iter().map(|arg| self.lower_expr(arg)).collect();
                    return self.emit_rvalue_to_temp(
                        AirType::Str,
                        Rvalue::Call {
                            func: Callee::Named("__aelys_str_from_char".to_string()),
                            args: lowered,
                        },
                        sp,
                    );
                }
                if enum_name == "char" && (variant == "from_i64" || variant == "is_scalar") {
                    let lowered: Vec<Operand> =
                        args.iter().map(|arg| self.lower_expr(arg)).collect();
                    let (symbol, ty) = if variant == "from_i64" {
                        ("__aelys_char_from_i64", AirType::Char)
                    } else {
                        ("__aelys_char_is_scalar", AirType::Bool)
                    };
                    return self.emit_rvalue_to_temp(
                        ty,
                        Rvalue::Call {
                            func: Callee::Named(symbol.to_string()),
                            args: lowered,
                        },
                        sp,
                    );
                }
                if enum_name == "string" && variant == "substring_bytes" {
                    let refs: Vec<&TypedExpr> = args.iter().collect();
                    let lowered = self.lower_pinned_operands(&refs, sp);
                    return self.emit_rvalue_to_temp(
                        AirType::Str,
                        Rvalue::Call {
                            func: Callee::Named("__aelys_str_substring_bytes".to_string()),
                            args: lowered,
                        },
                        sp,
                    );
                }
                let mut payload: Vec<Operand> = Vec::with_capacity(args.len());
                let values: Vec<&TypedExpr> = args.iter().collect();
                for (i, arg) in args.iter().enumerate() {
                    let op = self.lower_expr(arg);
                    let op = self.hold_operand(op, arg, &values[i + 1..], false, sp);
                    let op = self.own_str_for_store(op, &arg.ty, sp);
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
                let air_ty = self.lower_type_from_infer(&expr.ty);
                let enum_ref = match &air_ty {
                    AirType::Enum(r) => r.clone(),
                    _ => EnumRef::plain(enum_name.clone()),
                };
                self.emit_rvalue_to_temp(
                    air_ty,
                    Rvalue::EnumInit {
                        enum_ref,
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
                let mut result = self.lower_expr(tail);
                let mut owned_tail = None;
                if let Operand::Copy(id) | Operand::Move(id) = &result {
                    let id = *id;
                    if self
                        .str_locals
                        .iter()
                        .any(|(l, d)| *l == id && *d > scope_depth)
                    {
                        self.forget_str_local(id);
                        owned_tail = Some(id);
                    } else if self.str_operand_is_fresh(&result) {
                        self.fresh_str_results.insert(id);
                    } else if self.claim_owned_str_temp(&result) {
                        owned_tail = Some(id);
                    } else if self.counted(&tail.ty) && !self.last_block_is_terminated() {
                        let r = self.alloc_temp(self.counted_operand_type(&result));
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Local(r),
                                rvalue: Rvalue::Use(result.clone()),
                            },
                            None,
                        );
                        self.emit_str_retain(r, None);
                        owned_tail = Some(r);
                        result = Operand::Copy(r);
                    }
                }
                if !self.last_block_is_terminated() {
                    let deeper: Vec<LocalId> = self
                        .str_locals
                        .iter()
                        .filter(|(_, d)| *d > scope_depth)
                        .map(|(l, _)| *l)
                        .collect();
                    for id in deeper {
                        self.emit_str_release(id, None);
                    }
                }
                self.str_locals.retain(|(_, d)| *d <= scope_depth);
                // a result that may point into a deeper local keeps it open, or releasing it here would free what it points to
                if !self.affine_category(&tail.ty).is_managed() && !self.may_hold_a_view(&tail.ty) {
                    self.emit_scope_rc_releases(scope_depth);
                } else {
                    let kept = self.cow_locals.iter().filter(|(_, d)| *d > scope_depth);
                    self.cow_zombies.extend(kept.map(|(id, _)| *id));
                }
                if let Some(id) = owned_tail
                    && !self.last_block_is_terminated()
                {
                    self.stmt_str_temps.push((id, self.current_blocks.len()));
                }
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
                let func = self.lower_callee(callee, args);
                let lowered_args = self.lower_call_args(callee, args, sp);
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
                    let result = self.emit_rvalue_to_temp(
                        ret_ty,
                        Rvalue::Call {
                            func,
                            args: lowered_args,
                        },
                        sp,
                    );
                    self.pose_vec_call_result(&result);
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
            let result = self.emit_rvalue_to_temp(result_ty, Rvalue::Call { func, args }, sp);
            self.pose_vec_call_result(&result);
            result
        }
    }

    fn lower_assign_common(&mut self, name: &str, value: &TypedExpr, sp: Option<Span>) -> Operand {
        let mut val = self.lower_expr(value);
        let str_val = self.claim_str_init(&mut val);
        let kept_unretained = match self.lookup_local(name) {
            Some(id) => self.capture_slots.contains_key(&id),
            None => true,
        };
        if kept_unretained
            && self.counted(&value.ty)
            && !matches!(str_val, crate::lower::StrInit::Own)
        {
            val = self.retained_str_copy(val, sp);
        }
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
            let vec_claimed = slot_is_vec && matches!(str_val, crate::lower::StrInit::Own);
            if !vec_claimed {
                self.emit_vec_slot_acquire(slot_is_vec, Some(&value.kind), &val, sp);
            }
            if slot_is_vec {
                if is_capture {
                    self.emit_cow_release_through_ptr(Operand::Copy(id), sp);
                } else {
                    self.emit_cow_release(id, sp);
                }
            }
            // `s = s` must emit neither half: the release would free the buffer the retain then re-takes
            let str_self_assign = matches!(val, Operand::Copy(v) | Operand::Move(v) if v == id);
            let mut str_share_taken = false;
            if !is_capture && !str_self_assign && self.str_local_is_owned(id) {
                str_share_taken = !matches!(str_val, crate::lower::StrInit::Own);
                self.emit_str_release(id, sp);
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
            if str_share_taken {
                self.emit_str_retain(id, sp);
            }
            Operand::Copy(id)
        } else {
            // the global owns what it holds, and the value replacing it has taken its own share
            let old = self.counted(&value.ty).then(|| {
                self.emit_rvalue_to_temp(
                    self.lower_type_from_infer(&value.ty),
                    Rvalue::Call {
                        func: Callee::Named(format!("__aelys_global_get_{}", name)),
                        args: Vec::new(),
                    },
                    sp,
                )
            });
            self.emit(
                AirStmtKind::CallVoid {
                    func: Callee::Named(format!("__aelys_global_set_{}", name)),
                    args: vec![val],
                },
                sp,
            );
            if let Some(Operand::Copy(id) | Operand::Move(id)) = old {
                self.emit_str_release(id, sp);
            }
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
                self.report_ice(
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
                self.report_ice(
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
                    self.report_ice(
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
            let op = self.lower_expr(value);
            self.own_str_for_store(op, &value.ty, sp)
        } else {
            Operand::Const(AirConst::Null)
        };

        let Some(base) = self.projection_base_mode(object, PlaceMode::Store) else {
            self.report_ice(
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
            let current = if matches!(value.ty, InferType::String)
                && Self::roots_a_vec(&object.ty)
                && super::pin::may_free(rhs_expr)
            {
                self.pin_str_read(current, sp)
            } else {
                current
            };
            let rhs = self.lower_expr(rhs_expr);
            let air_op = super::lower_binop(&op);
            let combined =
                self.emit_rvalue_to_temp(elem_ty, Rvalue::BinaryOp(air_op, current, rhs), sp);
            self.own_str_for_store(combined, &value.ty, sp)
        } else {
            val
        };

        if Self::roots_a_vec(&object.ty) {
            self.emit_cow_detach(base.ptr, sp);
        }
        // the old value leaves only after the new one is in, so `v[0] = v[0]` keeps its string
        let owned_array = Self::roots_an_array(&object.ty) && self.store_root_owns(object);
        // a mutable slice is a unique borrow, so its slots keep one share each like an array's
        let drops_old =
            Self::roots_a_vec(&object.ty) || owned_array || Self::slice_is_mutable(&object.ty);
        let old_elem = (drops_old && self.counted(&value.ty)).then(|| {
            let elem_ty = self.lower_type_from_infer(&value.ty);
            self.emit_rvalue_to_temp(
                elem_ty,
                Rvalue::Index {
                    base: Operand::Copy(base.ptr),
                    index: idx.clone(),
                },
                sp,
            )
        });
        self.emit(
            AirStmtKind::Assign {
                place: Place::Index(base.ptr, idx),
                rvalue: Rvalue::Use(final_val),
            },
            sp,
        );
        if let Some(Operand::Copy(old) | Operand::Move(old)) = old_elem {
            self.emit_str_release(old, sp);
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
            let op = self.lower_expr(value);
            self.own_str_for_store(op, &value.ty, sp)
        } else {
            Operand::Const(AirConst::Null)
        };

        let Some(base) = self.projection_base_mode(object, PlaceMode::Store) else {
            self.report_ice(
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
            let combined =
                self.emit_rvalue_to_temp(field_ty, Rvalue::BinaryOp(air_op, current, rhs), sp);
            self.own_str_for_store(combined, &value.ty, sp)
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
            let old_field = (self.counted(&value.ty) && self.store_root_owns(object)).then(|| {
                let field_ty = self.lower_type_from_infer(&value.ty);
                self.emit_rvalue_to_temp(
                    field_ty,
                    Rvalue::FieldAccess {
                        base: Operand::Copy(base.ptr),
                        field: field.to_string(),
                    },
                    sp,
                )
            });
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Field(base.ptr, field.to_string()),
                    rvalue: Rvalue::Use(final_val),
                },
                sp,
            );
            if let Some(Operand::Copy(old) | Operand::Move(old)) = old_field {
                self.emit_str_release(old, sp);
            }
        }

        Operand::Const(AirConst::Null)
    }

    pub(super) fn named_callee(&self, callee: &TypedExpr) -> Option<Callee> {
        match &callee.kind {
            TypedExprKind::Identifier(name) => (self.lookup_local(name).is_none()
                && !self.is_global_name(name))
            .then(|| Callee::Named(name.clone())),
            TypedExprKind::Member { object, member } => match &object.kind {
                TypedExprKind::Identifier(mod_name) => {
                    let is_runtime_value = self.lookup_local(mod_name).is_some()
                        || self.globals.iter().any(|global| global.name == *mod_name);
                    (!is_runtime_value).then(|| Callee::Named(format!("{}.{}", mod_name, member)))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn lower_callee(&mut self, callee: &TypedExpr, args: &[TypedExpr]) -> Callee {
        if let Some(named) = self.named_callee(callee) {
            return named;
        }
        if let TypedExprKind::Identifier(name) = &callee.kind
            && let Some(id) = self.lookup_local(name)
            && !self.capture_slots.contains_key(&id)
        {
            if !args.iter().any(|a| super::pin::may_write_local(a, name)) {
                return Callee::FnPtr(id);
            }
            let copy = self.alloc_temp(self.lower_type_from_infer(&callee.ty));
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(copy),
                    rvalue: Rvalue::Use(Operand::Copy(id)),
                },
                None,
            );
            return Callee::FnPtr(copy);
        }
        let op = self.lower_expr(callee);
        let ty = self.lower_type_from_infer(&callee.ty);
        Callee::FnPtr(self.operand_to_local(op, &ty))
    }

    // a generic or external callee may keep an argument without retaining it, so it gets a share
    fn lower_call_args(
        &mut self,
        callee: &TypedExpr,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Vec<Operand> {
        let keeps_unretained = matches!(self.named_callee(callee), Some(Callee::Named(name))
            if !self.retaining_fns.contains(&name)
                && !crate::symbols::BOOTSTRAP_BUILTIN_SYMBOLS.contains(&name.as_str()));
        let refs: Vec<&TypedExpr> = args.iter().collect();
        self.lower_pinned_operands_owning(&refs, keeps_unretained, sp)
    }

    pub(super) fn retain_str_fill(&mut self, fill: &Operand, sp: Option<Span>) {
        if let Operand::Copy(id) | Operand::Move(id) = fill
            && self
                .local_air_type(*id)
                .is_some_and(|ty| self.counted_air(&ty))
        {
            self.emit_str_retain(*id, sp);
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

        if self.position_is_dead() {
            return Operand::Const(AirConst::Null);
        }

        let eval_right_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();
        let dominated = self.str_temps_posed_here();

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
        let before = self.str_temp_ids();
        let rhs = self.lower_expr(right);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(rhs),
            },
            None,
        );
        self.release_arm_str_temps(&before);
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(merge_id);
        self.restamp_str_temps(&dominated);
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
        let str_result = result.is_some() && self.counted(&parent.ty);

        let cond = self.lower_expr(condition);
        if self.position_is_dead() {
            return Operand::Const(AirConst::Null);
        }
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();
        let dominated = self.str_temps_posed_here();

        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: then_id,
            else_block: else_id,
        });

        self.fixup_block_id_noop(then_id);
        let before = self.str_temp_ids();
        if let Some(result) = result {
            let then_val = self.lower_expr(then_branch);
            self.store_arm_result(result, then_val, str_result);
        } else {
            self.lower_expr_discard(then_branch);
        }
        self.release_arm_str_temps(&before);
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(else_id);
        let before = self.str_temp_ids();
        if let Some(result) = result {
            let else_val = self.lower_expr(else_branch);
            self.store_arm_result(result, else_val, str_result);
        } else {
            self.lower_expr_discard(else_branch);
        }
        self.release_arm_str_temps(&before);
        self.seal_block(AirTerminator::Goto(merge_id));

        self.fixup_block_id_noop(merge_id);
        self.restamp_str_temps(&dominated);
        self.pose_owned_str_result(result, str_result);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
    }

    fn store_arm_result(&mut self, result: LocalId, mut val: Operand, str_result: bool) {
        let vec_result = matches!(self.local_air_type(result), Some(AirType::Vec(_)));
        let claimed = (str_result || vec_result) && self.claim_owned_str_temp(&val);
        if let (true, Operand::Copy(id)) = (claimed, &val) {
            val = Operand::Move(*id);
        }
        let owned = !(str_result || vec_result)
            || claimed
            || (str_result
                && (matches!(val, Operand::Const(_)) || self.str_operand_is_fresh(&val)));
        let reachable = !self.last_block_is_terminated();
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(val),
            },
            None,
        );
        if !owned && reachable {
            if vec_result {
                self.emit_cow_retain(result, None);
            } else {
                self.emit_str_retain(result, None);
            }
        }
    }

    fn pose_owned_str_result(&mut self, result: Option<LocalId>, str_result: bool) {
        if let Some(local) = result
            && (str_result || matches!(self.local_air_type(local), Some(AirType::Vec(_))))
        {
            self.stmt_str_temps.push((local, self.current_blocks.len()));
        }
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
            self.report_ice(format!(
                "ICE: Vec::push expects 2 arguments, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let vec_addr = match self.projection_base(&args[0]) {
            Some(addr) => Operand::Copy(addr.ptr),
            None => {
                self.report_ice(
                    "ICE: Vec::push target denotes no storage at AIR lowering; sema must \
                     reject it (E0421)"
                        .to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
        let elem_ty = self.lower_type_from_infer(&args[1].ty);
        let elem_op = self.lower_expr(&args[1]);
        let elem_op = self.own_str_for_store(elem_op, &args[1].ty, sp);
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

    fn lower_vec_pop(
        &mut self,
        result_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        if args.len() != 1 {
            self.report_ice(format!(
                "ICE: Vec::pop expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let vec_addr = match self.projection_base(&args[0]) {
            Some(addr) => Operand::Copy(addr.ptr),
            None => {
                self.report_ice(
                    "ICE: Vec::pop target denotes no storage at AIR lowering; sema must \
                     reject it (E0421)"
                        .to_string(),
                );
                return Operand::Const(AirConst::Null);
            }
        };
        let elem_ty = self.lower_type_from_infer(result_ty);
        let out_slot = self.alloc_temp_mut(elem_ty.clone());
        let out_addr = self.addr_of_own_temp(out_slot, &elem_ty, sp);
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_vec_pop".to_string()),
                args: vec![vec_addr, out_addr],
            },
            sp,
        );
        if self.counted_air(&elem_ty) && !self.last_block_is_terminated() {
            self.stmt_str_temps
                .push((out_slot, self.current_blocks.len()));
        }
        Operand::Copy(out_slot)
    }

    fn lower_len_member(&mut self, object: &TypedExpr, sp: Option<Span>) -> Operand {
        if let Some(addr) = self.projection_base(object) {
            return self.emit_rvalue_to_temp(
                AirType::I64,
                Rvalue::Len(Operand::Copy(addr.ptr)),
                sp,
            );
        }
        let base = self.lower_expr(object);
        self.emit_rvalue_to_temp(
            AirType::I64,
            Rvalue::FieldAccess {
                base,
                field: "len".to_string(),
            },
            sp,
        )
    }

    fn lower_vec_len(&mut self, args: &[TypedExpr], sp: Option<Span>) -> Operand {
        if args.len() != 1 {
            self.report_ice(format!(
                "ICE: Vec::len expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        self.lower_len_member(&args[0], sp)
    }

    fn lower_vec_as_slice(
        &mut self,
        result_ty: &InferType,
        args: &[TypedExpr],
        sp: Option<Span>,
    ) -> Operand {
        if args.len() != 1 {
            self.report_ice(format!(
                "ICE: Vec::as_slice expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let Some(addr) = self.projection_base(&args[0]) else {
            self.report_ice(
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
            self.report_ice(format!(
                "ICE: Vec::try_as_unique_mut_slice expects 1 argument, got {}",
                args.len()
            ));
            return Operand::Const(AirConst::Null);
        }
        let vec_addr = match self.projection_base(&args[0]) {
            Some(addr) => Operand::Copy(addr.ptr),
            None => {
                self.report_ice(
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
            .map(|a| {
                let op = self.lower_expr(a);
                self.own_str_for_store(op, &a.ty, sp)
            })
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
            bounds: Vec::new(),
            params: params.to_vec(),
            return_type: return_type.clone(),
            body: body.to_vec(),
            decorators: Vec::new(),
            is_pub: false,
            declared_nogc: false,
            foreign: None,
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
                self.report_unsupported(format!(
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
                    self.report_ice(format!(
                        "ICE: captured variable `{}` not found in scope during closure lowering",
                        cap_name
                    ));
                    continue;
                };
                let cap_val = self.own_str_for_store(cap_val, cap_ty, sp);
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

    // a fresh aggregate no binding holds is this frame's, released when the statement ends
    fn pose_fresh_temp(&mut self, operand: &Operand) {
        let (Operand::Copy(id) | Operand::Move(id)) = operand else {
            return;
        };
        if self.str_operand_is_fresh(operand) {
            self.stmt_str_temps.push((*id, self.current_blocks.len()));
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
        let str_result = result.is_some() && self.counted(&parent.ty);

        let scrutinee_op = self.lower_expr(scrutinee);
        if self.position_is_dead() {
            return Operand::Const(AirConst::Null);
        }
        self.pose_fresh_temp(&scrutinee_op);

        let enum_ref = match self.lower_type_from_infer(&scrutinee.ty) {
            AirType::Enum(r) => r,
            _ => {
                self.report_ice(format!(
                    "match scrutinee is not an enum type: {:?}",
                    scrutinee.ty
                ));
                return Operand::Const(AirConst::Null);
            }
        };

        let tag_op = self.emit_rvalue_to_temp(
            AirType::I32,
            Rvalue::EnumTag {
                enum_ref: enum_ref.clone(),
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
        let dominated = self.str_temps_posed_here();

        self.seal_block(AirTerminator::Switch {
            discr: tag_op,
            targets: switch_targets,
            default: default_id,
        });

        for (block_id, arm) in arm_blocks {
            self.fixup_block_id_noop(block_id);

            let arm_scope_depth = self.locals_by_name.len();
            let before = self.str_temp_ids();

            if let TypedPattern::Variant { tag, bindings, .. } = &arm.pattern {
                for (field_index, (name, ty)) in bindings.iter().enumerate() {
                    let field_ty = self.lower_type_from_infer(ty);
                    let field_local = self.alloc_named_local(name, field_ty.clone(), false, sp);
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(field_local),
                            rvalue: Rvalue::EnumPayload {
                                enum_ref: enum_ref.clone(),
                                tag: *tag,
                                operand: scrutinee_op.clone(),
                                field_index: field_index as u32,
                            },
                        },
                        sp,
                    );
                    if self.counted_air(&field_ty) {
                        self.emit_str_retain(field_local, sp);
                        self.str_locals
                            .push((field_local, self.locals_by_name.len()));
                    }
                }
            }

            if let Some(result) = result {
                let arm_val = self.lower_expr(&arm.body);
                self.store_arm_result(result, arm_val, str_result);
            } else {
                self.lower_expr_discard(&arm.body);
            }
            self.release_arm_str_temps(&before);
            if !self.last_block_is_terminated() {
                let binders: Vec<LocalId> = self
                    .str_locals
                    .iter()
                    .filter(|(_, d)| *d > arm_scope_depth)
                    .map(|(l, _)| *l)
                    .collect();
                for id in binders {
                    self.emit_str_release(id, None);
                }
            }
            self.str_locals.retain(|(_, d)| *d <= arm_scope_depth);
            self.locals_by_name.truncate(arm_scope_depth);
            self.seal_block(AirTerminator::Goto(merge_id));
        }

        self.fixup_block_id_noop(default_id);
        if let Some(wildcard) = wildcard_arm {
            let before = self.str_temp_ids();
            if let Some(result) = result {
                let wc_val = self.lower_expr(&wildcard.body);
                self.store_arm_result(result, wc_val, str_result);
            } else {
                self.lower_expr_discard(&wildcard.body);
            }
            self.release_arm_str_temps(&before);
            self.seal_block(AirTerminator::Goto(merge_id));
        } else {
            self.seal_block(AirTerminator::Unreachable);
        }

        self.fixup_block_id_noop(merge_id);
        self.restamp_str_temps(&dominated);
        self.pose_owned_str_result(result, str_result);
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
        if self.position_is_dead() {
            return Operand::Const(AirConst::Null);
        }
        self.pose_fresh_temp(&scrutinee_op);
        let enum_ref = match self.lower_type_from_infer(&scrutinee.ty) {
            AirType::Enum(r) => r,
            _ => {
                self.report_ice(format!(
                    "result assert scrutinee is not an enum type: {:?}",
                    scrutinee.ty
                ));
                return Operand::Const(AirConst::Null);
            }
        };

        let tag_op = self.emit_rvalue_to_temp(
            AirType::I32,
            Rvalue::EnumTag {
                enum_ref: enum_ref.clone(),
                operand: scrutinee_op.clone(),
            },
            sp,
        );

        let ok_block = self.alloc_block_id();
        let err_block = self.alloc_block_id();
        let merge = self.alloc_block_id();
        let dominated = self.str_temps_posed_here();

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
                        enum_ref,
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
        self.restamp_str_temps(&dominated);
        result.map_or(Operand::Const(AirConst::Null), Operand::Copy)
    }

    fn lower_fmt_string(&mut self, parts: &[TypedFmtStringPart], sp: Option<Span>) -> Operand {
        let mut operands: Vec<Operand> = Vec::new();
        let exprs: Vec<&TypedExpr> = parts
            .iter()
            .filter_map(|part| match part {
                TypedFmtStringPart::Expr(expr) => Some(expr.as_ref()),
                TypedFmtStringPart::Literal(_) | TypedFmtStringPart::Placeholder => None,
            })
            .collect();
        let mut lowered = self.lower_pinned_operands(&exprs, sp).into_iter();

        for part in parts {
            match part {
                TypedFmtStringPart::Literal(s) => {
                    operands.push(Operand::Const(AirConst::Str(s.clone())));
                }
                TypedFmtStringPart::Expr(expr) => {
                    let val = lowered.next().expect("one operand per part");
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

pub(super) fn non_constant_array_size_message(size: &TypedExpr) -> String {
    format!(
        "unsupported non-constant array size: an array size must be a compile-time constant, \
         and the size expression at line {}, column {} is not; annotate the binding `[T; N]` \
         if the length is known",
        size.span.line, size.span.column
    )
}

impl LoweringContext<'_> {
    // a type parameter or an unresolved type may become a view or a managed value after monomorphization
    fn may_hold_a_view(&self, ty: &InferType) -> bool {
        match ty {
            InferType::Ref { .. }
            | InferType::Slice { .. }
            | InferType::Var(_)
            | InferType::Dynamic => true,
            InferType::Struct(name) => self.type_params_map.iter().any(|(n, _)| n == name),
            InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
                self.may_hold_a_view(inner)
            }
            InferType::Tuple(items) | InferType::Enum(_, items) => {
                items.iter().any(|item| self.may_hold_a_view(item))
            }
            _ => false,
        }
    }
}
