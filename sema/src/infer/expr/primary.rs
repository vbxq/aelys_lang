use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};
use std::collections::HashMap;

impl TypeInference {
    pub(super) fn infer_identifier_expr(
        &mut self,
        name: &str,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let ty = self
            .env
            .lookup(name)
            .or_else(|| self.env.lookup_function_ref(name))
            .cloned()
            .unwrap_or_else(|| {
                self.errors
                    .push(TypeError::undefined_variable(name.to_string(), span));

                // register the variable with dynamic type to prevent repeated "undefined variable" errors for each subsequent use
                let recovery_ty = InferType::Dynamic;
                self.env.define_local(name.to_string(), recovery_ty.clone());

                recovery_ty
            });

        (TypedExprKind::Identifier(name.to_string()), ty)
    }

    pub(super) fn instantiate_enum_type_param(
        &mut self,
        ty: &InferType,
        type_param_names: &[String],
        mapping: &mut HashMap<String, InferType>,
    ) -> InferType {
        match ty {
            InferType::Struct(name) if type_param_names.contains(name) => mapping
                .entry(name.clone())
                .or_insert_with(|| self.type_gen.fresh())
                .clone(),
            InferType::Function { params, ret, nogc } => {
                let new_params = params
                    .iter()
                    .map(|p| self.instantiate_enum_type_param(p, type_param_names, mapping))
                    .collect();
                let new_ret =
                    Box::new(self.instantiate_enum_type_param(ret, type_param_names, mapping));
                InferType::Function {
                    params: new_params,
                    ret: new_ret,
                    nogc: *nogc,
                }
            }
            InferType::Array(inner, len) => InferType::Array(
                Box::new(self.instantiate_enum_type_param(inner, type_param_names, mapping)),
                *len,
            ),
            InferType::Vec(inner) => InferType::Vec(Box::new(self.instantiate_enum_type_param(
                inner,
                type_param_names,
                mapping,
            ))),
            InferType::Ref { referent, mutable } => InferType::Ref {
                referent: Box::new(self.instantiate_enum_type_param(
                    referent,
                    type_param_names,
                    mapping,
                )),
                mutable: *mutable,
            },
            InferType::Slice { elem, mutable } => InferType::Slice {
                elem: Box::new(self.instantiate_enum_type_param(elem, type_param_names, mapping)),
                mutable: *mutable,
            },
            InferType::Tuple(elems) => InferType::Tuple(
                elems
                    .iter()
                    .map(|e| self.instantiate_enum_type_param(e, type_param_names, mapping))
                    .collect(),
            ),
            InferType::Enum(name, args) => {
                let new_args = args
                    .iter()
                    .map(|a| self.instantiate_enum_type_param(a, type_param_names, mapping))
                    .collect();
                InferType::Enum(name.clone(), new_args)
            }
            other => other.clone(),
        }
    }

    pub(super) fn infer_enum_variant(
        &mut self,
        enum_name: &str,
        variant: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        // intercepted here, before the enum lookup that would never find them
        if enum_name == "Rc" && variant == "new" {
            return self.infer_rc_new(args, span);
        }
        if enum_name == "Rc" && variant == "get" {
            return self.infer_rc_get(args, span);
        }
        if enum_name == "Rc" && variant == "null" {
            return self.infer_rc_null(args, span);
        }
        if enum_name == "Vec" && variant == "new" {
            return self.infer_vec_new(args, span);
        }
        if enum_name == "Vec" && variant == "push" {
            return self.infer_vec_push(args, span);
        }
        if enum_name == "Vec" && variant == "try_as_unique_mut_slice" {
            return self.infer_vec_try_as_unique_mut_slice(args, span);
        }
        if enum_name == "Vec" && (variant == "len" || variant == "as_slice") {
            return self.infer_vec_read(variant, args, span);
        }

        let enum_def = self.type_table.get_enum(enum_name).cloned();
        match enum_def {
            Some(def) => {
                if let Some(v) = def.variants.iter().find(|v| v.name == variant) {
                    let expected_arity = v.data.len();
                    let actual_arity = args.len();

                    if expected_arity != actual_arity {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::ArityMismatch {
                                expected: expected_arity,
                                found: actual_arity,
                            },
                            span,
                            reason: ConstraintReason::Other(format!(
                                "enum variant '{}::{}' expects {} argument{}, got {}",
                                enum_name,
                                variant,
                                expected_arity,
                                if expected_arity == 1 { "" } else { "s" },
                                actual_arity,
                            )),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                        let typed_args: Vec<TypedExpr> =
                            args.iter().map(|a| self.infer_expr(a)).collect();
                        return (
                            TypedExprKind::EnumVariant {
                                enum_name: enum_name.to_string(),
                                variant: variant.to_string(),
                                tag: v.tag,
                                args: typed_args,
                            },
                            InferType::Dynamic,
                        );
                    }

                    let is_generic = !def.type_params.is_empty();
                    let mut type_param_mapping: HashMap<String, InferType> = HashMap::new();

                    let mut typed_args = Vec::with_capacity(args.len());
                    for (i, arg_expr) in args.iter().enumerate() {
                        let mut typed_arg = self.infer_expr(arg_expr);

                        self.reject_rc_out_of_carrier_surface(
                            &typed_arg.ty,
                            is_generic,
                            arg_expr.span,
                            &format!("payload {i} of enum variant `{enum_name}::{variant}`"),
                        );

                        let expected_ty = if is_generic {
                            self.instantiate_enum_type_param(
                                &v.data[i],
                                &def.type_params,
                                &mut type_param_mapping,
                            )
                        } else {
                            v.data[i].clone()
                        };

                        self.try_narrow_literal(&mut typed_arg, &expected_ty);

                        if typed_arg.ty != expected_ty
                            && typed_arg.ty.can_implicit_widen_to(&expected_ty)
                        {
                            let vspan = typed_arg.span;
                            let original = std::mem::replace(
                                &mut typed_arg,
                                TypedExpr {
                                    kind: TypedExprKind::Null,
                                    ty: InferType::Null,
                                    span: vspan,
                                },
                            );
                            typed_arg = TypedExpr {
                                kind: TypedExprKind::Cast {
                                    expr: Box::new(original),
                                    target: expected_ty.clone(),
                                },
                                ty: expected_ty.clone(),
                                span: vspan,
                            };
                        }

                        self.constraints.push(Constraint::flows(
                            typed_arg.ty.clone(),
                            expected_ty,
                            arg_expr.span,
                            ConstraintReason::Other(format!(
                                "argument {} of enum variant '{}::{}'",
                                i, enum_name, variant,
                            )),
                        ));

                        typed_args.push(typed_arg);
                    }

                    // lowering can pre-mangle the enum name for monomorphization.
                    let type_args: Vec<InferType> = if is_generic {
                        def.type_params
                            .iter()
                            .map(|param| {
                                type_param_mapping
                                    .get(param)
                                    .cloned()
                                    .unwrap_or_else(|| self.type_gen.fresh())
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let result_ty = InferType::Enum(enum_name.to_string(), type_args);

                    (
                        TypedExprKind::EnumVariant {
                            enum_name: enum_name.to_string(),
                            variant: variant.to_string(),
                            tag: v.tag,
                            args: typed_args,
                        },
                        result_ty,
                    )
                } else {
                    let variant_names: Vec<_> =
                        def.variants.iter().map(|v| v.name.as_str()).collect();
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: InferType::Enum(enum_name.to_string(), Vec::new()),
                            found: InferType::Dynamic,
                        },
                        span,
                        reason: ConstraintReason::Other(format!(
                            "unknown variant '{}' on enum '{}'; known variants: {}",
                            variant,
                            enum_name,
                            variant_names.join(", ")
                        )),
                        secondary_spans: Vec::new(),
                        help: None,
                        suggestion: None,
                    });
                    (TypedExprKind::Null, InferType::Dynamic)
                }
            }
            None => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Dynamic,
                        found: InferType::Dynamic,
                    },
                    span,
                    reason: ConstraintReason::UnknownType {
                        name: enum_name.to_string(),
                    },
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                (TypedExprKind::Null, InferType::Dynamic)
            }
        }
    }

    fn infer_rc_new(&mut self, args: &[Expr], span: Span) -> (TypedExprKind, InferType) {
        if args.len() != 1 {
            self.errors.push(TypeError::rc_out_of_surface(
                format!("Rc::new expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            // still type the args, so downstream inference stays stable
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Rc".to_string(),
                    variant: "new".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }

        let typed_arg = self.infer_expr(&args[0]);
        let inner = typed_arg.ty.clone();
        (
            TypedExprKind::EnumVariant {
                enum_name: "Rc".to_string(),
                variant: "new".to_string(),
                tag: 0,
                args: vec![typed_arg],
            },
            InferType::Rc(Box::new(inner)),
        )
    }

    fn infer_rc_get(&mut self, args: &[Expr], span: Span) -> (TypedExprKind, InferType) {
        if args.len() != 1 {
            self.errors.push(TypeError::rc_out_of_surface(
                format!("Rc::get expects exactly 1 argument, got {}", args.len()),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Rc".to_string(),
                    variant: "get".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }

        let typed_arg = self.infer_expr(&args[0]);
        let result_ty = match &typed_arg.ty {
            InferType::Rc(inner) => inner.as_ref().clone(),
            other => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Rc(Box::new(InferType::Dynamic)),
                        found: other.clone(),
                    },
                    span,
                    reason: ConstraintReason::Other(format!(
                        "Rc::get expects an `Rc<T>` argument, got `{other}`"
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                InferType::Dynamic
            }
        };
        (
            TypedExprKind::EnumVariant {
                enum_name: "Rc".to_string(),
                variant: "get".to_string(),
                tag: 0,
                args: vec![typed_arg],
            },
            result_ty,
        )
    }

    fn infer_rc_null(&mut self, args: &[Expr], span: Span) -> (TypedExprKind, InferType) {
        if !args.is_empty() {
            self.errors.push(TypeError::rc_out_of_surface(
                format!("Rc::null expects 0 arguments, got {}", args.len()),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Rc".to_string(),
                    variant: "null".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }
        let inner = self.type_gen.fresh();
        (
            TypedExprKind::EnumVariant {
                enum_name: "Rc".to_string(),
                variant: "null".to_string(),
                tag: 0,
                args: vec![],
            },
            InferType::Rc(Box::new(inner)),
        )
    }

    fn infer_vec_new(&mut self, args: &[Expr], span: Span) -> (TypedExprKind, InferType) {
        if !args.is_empty() {
            self.errors.push(TypeError::rc_out_of_surface(
                format!("Vec::new expects 0 arguments, got {}", args.len()),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Vec".to_string(),
                    variant: "new".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }
        let elem = self.type_gen.fresh();
        (
            TypedExprKind::EnumVariant {
                enum_name: "Vec".to_string(),
                variant: "new".to_string(),
                tag: 0,
                args: vec![],
            },
            InferType::Vec(Box::new(elem)),
        )
    }

    fn infer_vec_push(&mut self, args: &[Expr], span: Span) -> (TypedExprKind, InferType) {
        if args.len() != 2 {
            self.errors.push(TypeError::rc_out_of_surface(
                format!("Vec::push expects exactly 2 arguments, got {}", args.len()),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Vec".to_string(),
                    variant: "push".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Null,
            );
        }
        let typed_vec = self.infer_expr(&args[0]);
        let typed_elem = self.infer_expr(&args[1]);

        match &typed_vec.ty {
            InferType::Vec(inner) => {
                self.constraints.push(Constraint::flows(
                    typed_elem.ty.clone(),
                    (**inner).clone(),
                    args[1].span,
                    ConstraintReason::ArrayElement,
                ));
            }
            InferType::Var(_) | InferType::Dynamic => {
                self.constraints.push(Constraint::equal(
                    typed_vec.ty.clone(),
                    InferType::Vec(Box::new(typed_elem.ty.clone())),
                    args[0].span,
                    ConstraintReason::ArrayElement,
                ));
            }
            other => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Vec(Box::new(InferType::Dynamic)),
                        found: other.clone(),
                    },
                    span,
                    reason: ConstraintReason::Other(format!(
                        "Vec::push expects a `Vec<T>` first argument, got `{other}`"
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
            }
        }

        if self.type_table.contains_rc_nominal(&typed_elem.ty) {
            self.errors.push(TypeError::rc_out_of_surface(
                format!(
                    "element pushed to a Vec has type `{}` which embeds an `Rc<T>`; \
                     storing an Rc (directly or inside a struct/enum) in a Vec is not supported yet",
                    typed_elem.ty
                ),
                args[1].span,
            ));
        }

        // the copy-on-write path memcpys elements flat, so an element that owns a vec buffer
        if self.type_table.contains_vec_by_value(&typed_elem.ty) {
            self.errors.push(TypeError::vec_out_of_surface(
                format!(
                    "element pushed to a Vec has type `{}`, which holds a `Vec<T>` by value; \
                     a Vec inside a Vec/array is not supported yet (the buffer would be shared \
                     without a retain, the transitive Vec retain/release is not implemented)",
                    typed_elem.ty
                ),
                args[1].span,
            ));
        }

        (
            TypedExprKind::EnumVariant {
                enum_name: "Vec".to_string(),
                variant: "push".to_string(),
                tag: 0,
                args: vec![typed_vec, typed_elem],
            },
            InferType::Null,
        )
    }

    fn infer_vec_read(
        &mut self,
        variant: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if args.len() != 1 {
            self.errors.push(TypeError::rc_out_of_surface(
                format!(
                    "Vec::{} expects exactly 1 argument, got {}",
                    variant,
                    args.len()
                ),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Vec".to_string(),
                    variant: variant.to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }

        let typed_vec = self.infer_expr(&args[0]);
        let inner = match &typed_vec.ty {
            InferType::Vec(inner) => inner.as_ref().clone(),
            InferType::Var(_) | InferType::Dynamic => {
                let elem = self.type_gen.fresh();
                self.constraints.push(Constraint::equal(
                    typed_vec.ty.clone(),
                    InferType::Vec(Box::new(elem.clone())),
                    args[0].span,
                    ConstraintReason::ArrayElement,
                ));
                elem
            }
            other => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Vec(Box::new(InferType::Dynamic)),
                        found: other.clone(),
                    },
                    span: args[0].span,
                    reason: ConstraintReason::Other(format!(
                        "Vec::{} expects a `Vec<T>` argument, got `{other}`",
                        variant
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                return (
                    TypedExprKind::EnumVariant {
                        enum_name: "Vec".to_string(),
                        variant: variant.to_string(),
                        tag: 0,
                        args: vec![typed_vec],
                    },
                    InferType::Dynamic,
                );
            }
        };
        let result_ty = if variant == "len" {
            InferType::I64
        } else {
            InferType::Slice {
                elem: Box::new(inner),
                mutable: false,
            }
        };
        (
            TypedExprKind::EnumVariant {
                enum_name: "Vec".to_string(),
                variant: variant.to_string(),
                tag: 0,
                args: vec![typed_vec],
            },
            result_ty,
        )
    }

    fn infer_vec_try_as_unique_mut_slice(
        &mut self,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        if args.len() != 1 {
            self.errors.push(TypeError::rc_out_of_surface(
                format!(
                    "Vec::try_as_unique_mut_slice expects exactly 1 argument, got {}",
                    args.len()
                ),
                span,
            ));
            let typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();
            return (
                TypedExprKind::EnumVariant {
                    enum_name: "Vec".to_string(),
                    variant: "try_as_unique_mut_slice".to_string(),
                    tag: 0,
                    args: typed_args,
                },
                InferType::Dynamic,
            );
        }

        let typed_vec = self.infer_expr(&args[0]);
        self.check_write_target(&typed_vec, "Vec::try_as_unique_mut_slice", args[0].span);
        let inner = match &typed_vec.ty {
            InferType::Vec(inner) => inner.as_ref().clone(),
            InferType::Var(_) | InferType::Dynamic => {
                let elem = self.type_gen.fresh();
                self.constraints.push(Constraint::equal(
                    typed_vec.ty.clone(),
                    InferType::Vec(Box::new(elem.clone())),
                    args[0].span,
                    ConstraintReason::ArrayElement,
                ));
                elem
            }
            other => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Vec(Box::new(InferType::Dynamic)),
                        found: other.clone(),
                    },
                    span: args[0].span,
                    reason: ConstraintReason::Other(format!(
                        "Vec::try_as_unique_mut_slice expects a `Vec<T>` argument, got `{other}`"
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                InferType::Dynamic
            }
        };

        (
            TypedExprKind::EnumVariant {
                enum_name: "Vec".to_string(),
                variant: "try_as_unique_mut_slice".to_string(),
                tag: 0,
                args: vec![typed_vec],
            },
            InferType::Slice {
                elem: Box::new(inner),
                mutable: true,
            },
        )
    }
}
