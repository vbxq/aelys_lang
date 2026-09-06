use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::typed_ast::{ResultAssertOnErr, TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, MatchArm, Pattern, Span};
use std::collections::HashMap;

impl TypeInference {
    fn instantiate_type_params(&mut self, ty: &InferType) -> InferType {
        let mut mapping: HashMap<String, InferType> = HashMap::new();
        self.instantiate_inner(ty, &mut mapping)
    }

    fn instantiate_inner(
        &mut self,
        ty: &InferType,
        mapping: &mut HashMap<String, InferType>,
    ) -> InferType {
        match ty {
            InferType::Struct(name) if !self.type_table.has_struct(name) => mapping
                .entry(name.clone())
                .or_insert_with(|| self.type_gen.fresh())
                .clone(),
            InferType::Function { params, ret, nogc } => {
                let new_params = params
                    .iter()
                    .map(|p| self.instantiate_inner(p, mapping))
                    .collect();
                let new_ret = Box::new(self.instantiate_inner(ret, mapping));
                InferType::Function {
                    params: new_params,
                    ret: new_ret,
                    nogc: *nogc,
                }
            }
            InferType::Array(inner, len) => {
                InferType::Array(Box::new(self.instantiate_inner(inner, mapping)), *len)
            }
            InferType::Vec(inner) => {
                InferType::Vec(Box::new(self.instantiate_inner(inner, mapping)))
            }
            InferType::Tuple(elems) => InferType::Tuple(
                elems
                    .iter()
                    .map(|e| self.instantiate_inner(e, mapping))
                    .collect(),
            ),
            InferType::Enum(name, type_args) if !type_args.is_empty() => InferType::Enum(
                name.clone(),
                type_args
                    .iter()
                    .map(|a| self.instantiate_inner(a, mapping))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn arg_is_nogc_ref(&self, arg: &Expr) -> bool {
        if let ExprKind::Member { object, member } = &arg.kind
            && let ExprKind::Identifier(namespace) = &object.kind
            && self.is_module_namespace(namespace)
        {
            return self
                .module_imports
                .namespaces
                .get(namespace)
                .and_then(|exports| match exports.value(member) {
                    crate::modules::Lookup::Found(item) => Some(item),
                    _ => None,
                })
                .is_some_and(|item| matches!(item.ty, InferType::Function { nogc: true, .. }));
        }
        let ExprKind::Identifier(name) = &arg.kind else {
            return false;
        };
        if let Some(local_ty) = self.env.lookup_local(name) {
            return self.nogc_fn_params.contains(name)
                && matches!(local_ty, InferType::Function { nogc: true, .. });
        }
        matches!(
            self.env.lookup_function_ref(name),
            Some(InferType::Function { nogc: true, .. })
        )
    }

    fn member_names_a_module(&self, object: &Expr) -> bool {
        match &object.kind {
            ExprKind::Identifier(name) => self.is_module_namespace(name),
            _ => false,
        }
    }

    pub(super) fn infer_call_expr(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if let ExprKind::Identifier(name) = &callee.kind
            && self.unsafe_depth == 0
            && self.foreign_sigs.contains(name)
            && !self.foreign_sigs_shadowed_by_local(name)
        {
            self.errors
                .push(TypeError::foreign_call_outside_unsafe(name, span));
        }
        let typed_callee = if let ExprKind::Member { object, member } = &callee.kind
            && matches!(
                member.as_str(),
                "unwrap" | "expect" | "map_error" | "into_ok" | "unwrap_unchecked"
            )
            && !self.member_names_a_module(object)
        {
            let typed_object = self.infer_expr(object);
            if let InferType::Enum(name, targs) = &typed_object.ty
                && name == "Result"
                && targs.len() == 2
            {
                let payload_ty = targs[0].clone();
                return self.infer_result_builtin(member, typed_object, payload_ty, args, span);
            }
            let member_ty = self.member_result_type(&typed_object.ty, member);
            TypedExpr::new(
                TypedExprKind::Member {
                    object: Box::new(typed_object),
                    member: member.clone(),
                },
                member_ty,
                callee.span,
            )
        } else {
            self.infer_expr(callee)
        };
        let mut typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();

        let ret_type = if matches!(typed_callee.ty, InferType::Dynamic) {
            InferType::Dynamic
        } else {
            let callee_ty = self.instantiate_type_params(&typed_callee.ty);

            if let InferType::Function { params, .. } = &callee_ty
                && params.len() == typed_args.len()
            {
                for (arg, param_ty) in typed_args.iter_mut().zip(params.iter()) {
                    self.try_narrow_literal(arg, param_ty);
                }

                for (arg, param_ty) in typed_args.iter_mut().zip(params.iter()) {
                    if arg.ty != *param_ty && arg.ty.can_implicit_widen_to(param_ty) {
                        let span = arg.span;
                        let original = std::mem::replace(
                            arg,
                            TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span,
                            },
                        );
                        *arg = TypedExpr {
                            kind: TypedExprKind::Cast {
                                expr: Box::new(original),
                                target: param_ty.clone(),
                            },
                            ty: param_ty.clone(),
                            span,
                        };
                    }
                }

                for (arg, param_ty) in args.iter().zip(params.iter()) {
                    if matches!(param_ty, InferType::Function { nogc: true, .. })
                        && !self.arg_is_nogc_ref(arg)
                    {
                        self.errors.push(TypeError::nogc_callback_mismatch(
                            &describe_callback_arg(arg),
                            arg.span,
                        ));
                    }
                }
            }

            // directly so that downstream expressions (e.g., match) can inspect
            let ret = if let InferType::Function { ret: fn_ret, .. } = &callee_ty {
                *fn_ret.clone()
            } else {
                self.type_gen.fresh()
            };

            let arg_types: Vec<InferType> = typed_args.iter().map(|a| a.ty.clone()).collect();
            let expected_fn_type = InferType::Function {
                params: arg_types,
                ret: Box::new(ret.clone()),
                nogc: false,
            };

            self.constraints.push(Constraint::flows_into(
                callee_ty,
                expected_fn_type,
                span,
                ConstraintReason::Other("function call".to_string()),
            ));

            ret
        };

        (
            TypedExprKind::Call {
                callee: Box::new(typed_callee),
                args: typed_args,
            },
            ret_type,
        )
    }

    fn infer_result_builtin(
        &mut self,
        member: &str,
        typed_object: TypedExpr,
        payload_ty: InferType,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if member == "map_error" {
            return self.infer_map_error(typed_object, payload_ty, args, span);
        }

        let ok_tag = match self
            .type_table
            .get_enum("Result")
            .and_then(|def| def.variants.iter().find(|v| v.name == "Ok"))
        {
            Some(v) => v.tag,
            None => {
                self.errors.push(TypeError::member_access(
                    format!(
                        "[eh-stage3] `.{member}()` needs a declared `enum Result<T, E> {{ Ok(T), Err(E) }}`"
                    ),
                    span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let on_err = match member {
            "unwrap" => {
                if !args.is_empty() {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "[eh-stage3] `.unwrap()` takes no arguments, found {}",
                            args.len()
                        ),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                ResultAssertOnErr::Panic("called .unwrap() on an Err value".to_string())
            }
            "expect" => {
                if args.len() != 1 {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "[eh-stage3] `.expect()` takes one string literal argument, found {}",
                            args.len()
                        ),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                match &args[0].kind {
                    ExprKind::String(msg) => ResultAssertOnErr::Panic(msg.clone()),
                    _ => {
                        self.errors.push(TypeError::member_access(
                            "[eh-stage3] `.expect(...)` takes a string literal message in V1"
                                .to_string(),
                            args[0].span,
                        ));
                        return (TypedExprKind::Null, InferType::Dynamic);
                    }
                }
            }
            // into_ok is a compile-time proof that the error is uninhabited, so err seals unreachable
            "into_ok" => {
                if !args.is_empty() {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "[eh-stage3] `.into_ok()` takes no arguments, found {}",
                            args.len()
                        ),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                let e = match &typed_object.ty {
                    InferType::Enum(name, targs) if name == "Result" && targs.len() == 2 => {
                        targs[1].clone()
                    }
                    _ => unreachable!("the seam only routes a Result<T, E> receiver here"),
                };
                if !matches!(e, InferType::Never) {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "[eh-stage3] `.into_ok()` requires `Result<T, Never>`; this Result can fail with error type `{e}`"
                        ),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                ResultAssertOnErr::Unreachable
            }
            "unwrap_unchecked" => {
                if !args.is_empty() {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "[eh-stage3] `.unwrap_unchecked()` takes no arguments, found {}",
                            args.len()
                        ),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                if self.unsafe_depth == 0 {
                    self.errors.push(TypeError::member_access(
                        "[eh-stage3] `.unwrap_unchecked()` requires an `unsafe` block".to_string(),
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
                ResultAssertOnErr::Unreachable
            }
            _ => unreachable!("the seam only routes unwrap/expect/into_ok/unwrap_unchecked here"),
        };

        (
            TypedExprKind::ResultAssert {
                scrutinee: Box::new(typed_object),
                ok_tag,
                payload_ty: payload_ty.clone(),
                on_err,
            },
            payload_ty,
        )
    }

    fn infer_map_error(
        &mut self,
        typed_object: TypedExpr,
        t: InferType,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if args.len() != 1 {
            self.errors.push(TypeError::member_access(
                format!(
                    "[eh-stage3] `.map_error()` takes one function argument, found {}",
                    args.len()
                ),
                span,
            ));
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        let e = match &typed_object.ty {
            InferType::Enum(name, targs) if name == "Result" && targs.len() == 2 => {
                targs[1].clone()
            }
            _ => unreachable!("the seam only routes a Result<T, E> receiver here"),
        };

        let typed_f = self.infer_expr(&args[0]);
        let e2 = match &typed_f.ty {
            InferType::Function { params, ret, .. } if params.len() == 1 => (**ret).clone(),
            InferType::Dynamic => return (TypedExprKind::Null, InferType::Dynamic),
            other => {
                self.errors.push(TypeError::member_access(
                    format!(
                        "[eh-stage3] `.map_error(f)` needs `f: fn({e}) -> E2`, found `{other}`"
                    ),
                    args[0].span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let obj_span = typed_object.span;
        let v_name = self.next_map_error_binding('v');
        let e_name = self.next_map_error_binding('e');

        let ok_arm = MatchArm {
            pattern: variant_pattern("Result", "Ok", vec![v_name.clone()], span),
            body: Box::new(variant_construct(
                "Result",
                "Ok",
                vec![ident_expr(&v_name, span)],
                span,
            )),
            span,
        };
        let err_arm = MatchArm {
            pattern: variant_pattern("Result", "Err", vec![e_name.clone()], span),
            body: Box::new(variant_construct(
                "Result",
                "Err",
                vec![call_expr(
                    args[0].clone(),
                    vec![ident_expr(&e_name, span)],
                    span,
                )],
                span,
            )),
            span,
        };

        let arms = vec![ok_arm, err_arm];
        let (kind, _) = self.infer_match_typed(typed_object, obj_span, &arms, span, false);
        (kind, InferType::Enum("Result".to_string(), vec![t, e2]))
    }

    // $ is rejected by the scanner, so these hygienic names never collide with user code
    fn next_map_error_binding(&mut self, tag: char) -> String {
        let n = self.try_counter;
        self.try_counter += 1;
        format!("__{tag}${n}")
    }
}

fn describe_callback_arg(arg: &Expr) -> String {
    match &arg.kind {
        ExprKind::Identifier(name) => format!("`{name}`"),
        ExprKind::Lambda { .. } => "a lambda".to_string(),
        ExprKind::Call { .. } => "a call result".to_string(),
        ExprKind::Index { .. } => "an index expression".to_string(),
        ExprKind::If { .. } => "a conditional expression".to_string(),
        ExprKind::Match { .. } => "a match expression".to_string(),
        _ => "an expression that is not a direct function reference".to_string(),
    }
}

fn variant_pattern(enum_name: &str, variant: &str, bindings: Vec<String>, span: Span) -> Pattern {
    Pattern::Variant {
        enum_name: enum_name.to_string(),
        variant: variant.to_string(),
        bindings,
        span,
    }
}

fn ident_expr(name: &str, span: Span) -> Expr {
    Expr::new(ExprKind::Identifier(name.to_string()), span)
}

fn variant_construct(enum_name: &str, variant: &str, args: Vec<Expr>, span: Span) -> Expr {
    Expr::new(
        ExprKind::EnumVariant {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            args,
        },
        span,
    )
}

fn call_expr(callee: Expr, args: Vec<Expr>, span: Span) -> Expr {
    Expr::new(
        ExprKind::Call {
            callee: Box::new(callee),
            args,
        },
        span,
    )
}
