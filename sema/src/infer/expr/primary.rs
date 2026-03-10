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

                // register the variable with Dynamic type to prevent repeated "undefined variable" errors for each subsequent use
                let recovery_ty = InferType::Dynamic;
                self.env.define_local(name.to_string(), recovery_ty.clone());

                recovery_ty
            });

        (TypedExprKind::Identifier(name.to_string()), ty)
    }

    /// Instantiate type parameter placeholders in an InferType.
    ///
    /// Replaces `Struct("T")` (where T is a known type param name) with fresh type vars.
    /// Uses a shared mapping so the same param name maps to the same fresh var within
    /// a single instantiation context.
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
            InferType::Function { params, ret } => {
                let new_params = params
                    .iter()
                    .map(|p| self.instantiate_enum_type_param(p, type_param_names, mapping))
                    .collect();
                let new_ret =
                    Box::new(self.instantiate_enum_type_param(ret, type_param_names, mapping));
                InferType::Function {
                    params: new_params,
                    ret: new_ret,
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
            InferType::Tuple(elems) => InferType::Tuple(
                elems
                    .iter()
                    .map(|e| self.instantiate_enum_type_param(e, type_param_names, mapping))
                    .collect(),
            ),
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

                    // For generic enums, instantiate type params with fresh type vars
                    let is_generic = !def.type_params.is_empty();
                    let mut type_param_mapping: HashMap<String, InferType> = HashMap::new();

                    // Type-check each argument against the expected data type
                    let mut typed_args = Vec::with_capacity(args.len());
                    for (i, arg_expr) in args.iter().enumerate() {
                        let mut typed_arg = self.infer_expr(arg_expr);

                        // Instantiate type params in the expected type if this is a generic enum
                        let expected_ty = if is_generic {
                            self.instantiate_enum_type_param(
                                &v.data[i],
                                &def.type_params,
                                &mut type_param_mapping,
                            )
                        } else {
                            v.data[i].clone()
                        };

                        // Try literal narrowing first
                        self.try_narrow_literal(&mut typed_arg, &expected_ty);

                        // Push a constraint: arg type == expected field type
                        self.constraints.push(Constraint::equal(
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

                    // For generic enums, the result type is Enum("Option") but we need
                    // to also create a fresh type var for the overall enum type so that
                    // it unifies with type annotations like `Option<i64>`.
                    // The enum type itself is always Enum(enum_name).
                    let result_ty = InferType::Enum(enum_name.to_string());

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
