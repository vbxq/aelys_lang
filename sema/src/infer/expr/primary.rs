use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

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

                // register the variable with Dynamic type to prevent repeated "undefined variable" errors for each subsequent use
                let recovery_ty = InferType::Dynamic;
                self.env.define_local(name.to_string(), recovery_ty.clone());

                recovery_ty
            });

        (TypedExprKind::Identifier(name.to_string()), ty)
    }

    pub(super) fn infer_enum_variant(
        &mut self,
        enum_name: &str,
        variant: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
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
                        // still produce a typed expression for recovery
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

                    // Type-check each argument against the expected data type
                    let mut typed_args = Vec::with_capacity(args.len());
                    for (i, arg_expr) in args.iter().enumerate() {
                        let mut typed_arg = self.infer_expr(arg_expr);
                        let expected_ty = &v.data[i];

                        // Try literal narrowing first
                        self.try_narrow_literal(&mut typed_arg, expected_ty);

                        // Push a constraint: arg type == expected field type
                        self.constraints.push(Constraint::equal(
                            typed_arg.ty.clone(),
                            expected_ty.clone(),
                            arg_expr.span,
                            ConstraintReason::Other(format!(
                                "argument {} of enum variant '{}::{}'",
                                i, enum_name, variant,
                            )),
                        ));

                        typed_args.push(typed_arg);
                    }

                    (
                        TypedExprKind::EnumVariant {
                            enum_name: enum_name.to_string(),
                            variant: variant.to_string(),
                            tag: v.tag,
                            args: typed_args,
                        },
                        InferType::Enum(enum_name.to_string()),
                    )
                } else {
                    let variant_names: Vec<_> =
                        def.variants.iter().map(|v| v.name.as_str()).collect();
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: InferType::Enum(enum_name.to_string()),
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
}
