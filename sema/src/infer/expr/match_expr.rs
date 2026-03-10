use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExprKind, TypedMatchArm, TypedPattern};
use crate::types::InferType;
use aelys_syntax::{Expr, MatchArm, Pattern, Span};
use std::collections::{HashMap, HashSet};

impl TypeInference {
    pub(super) fn infer_match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_scrutinee = self.infer_expr(scrutinee);

        // The scrutinee must be an enum type
        let enum_name = match &typed_scrutinee.ty {
            InferType::Enum(name) => name.clone(),
            InferType::Dynamic => {
                // Error recovery: type-check arms but don't validate patterns
                let typed_arms = self.infer_match_arms_dynamic(arms);
                let result_type = if typed_arms.is_empty() {
                    InferType::Dynamic
                } else {
                    typed_arms[0].body.ty.clone()
                };
                return (
                    TypedExprKind::Match {
                        scrutinee: Box::new(typed_scrutinee),
                        arms: typed_arms,
                    },
                    result_type,
                );
            }
            other => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Dynamic,
                        found: other.clone(),
                    },
                    span: scrutinee.span,
                    reason: ConstraintReason::Other(format!(
                        "match scrutinee must be an enum type, got {}",
                        other
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let enum_def = self.type_table.get_enum(&enum_name).cloned();
        let enum_def = match enum_def {
            Some(def) => def,
            None => {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Dynamic,
                        found: InferType::Dynamic,
                    },
                    span,
                    reason: ConstraintReason::UnknownType {
                        name: enum_name.clone(),
                    },
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let mut typed_arms = Vec::new();
        let mut covered_variants: HashSet<String> = HashSet::new();
        let mut has_wildcard = false;

        // Create a fresh type variable for the result type
        let result_type = self.type_gen.fresh();

        // For generic enums, create a shared type param mapping across all arms
        // so the same type param resolves to the same type var in all arms.
        let is_generic = !enum_def.type_params.is_empty();
        let mut type_param_mapping: HashMap<String, InferType> = HashMap::new();

        for arm in arms {
            match &arm.pattern {
                Pattern::Variant {
                    enum_name: pat_enum,
                    variant,
                    bindings,
                    span: pat_span,
                } => {
                    // Verify enum name matches scrutinee
                    if *pat_enum != enum_name {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Enum(enum_name.clone()),
                                found: InferType::Enum(pat_enum.clone()),
                            },
                            span: *pat_span,
                            reason: ConstraintReason::Other(format!(
                                "pattern enum '{}' does not match scrutinee enum '{}'",
                                pat_enum, enum_name
                            )),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                        continue;
                    }

                    // Look up the variant
                    let variant_def = enum_def.variants.iter().find(|v| v.name == *variant);
                    let variant_def = match variant_def {
                        Some(v) => v,
                        None => {
                            let variant_names: Vec<_> =
                                enum_def.variants.iter().map(|v| v.name.as_str()).collect();
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::Mismatch {
                                    expected: InferType::Enum(enum_name.clone()),
                                    found: InferType::Dynamic,
                                },
                                span: *pat_span,
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
                            continue;
                        }
                    };

                    // Check for duplicate variant patterns
                    if covered_variants.contains(variant) {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Enum(enum_name.clone()),
                                found: InferType::Dynamic,
                            },
                            span: *pat_span,
                            reason: ConstraintReason::Other(format!(
                                "duplicate pattern for variant '{}::{}'",
                                enum_name, variant
                            )),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                    }
                    covered_variants.insert(variant.clone());

                    // Check binding count matches variant data fields
                    let expected_bindings = variant_def.data.len();
                    let actual_bindings = bindings.len();
                    if expected_bindings != actual_bindings {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::ArityMismatch {
                                expected: expected_bindings,
                                found: actual_bindings,
                            },
                            span: *pat_span,
                            reason: ConstraintReason::Other(format!(
                                "variant '{}::{}' has {} data field{}, but pattern has {} binding{}",
                                enum_name,
                                variant,
                                expected_bindings,
                                if expected_bindings == 1 { "" } else { "s" },
                                actual_bindings,
                                if actual_bindings == 1 { "" } else { "s" },
                            )),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                    }

                    // Introduce bindings as locals and infer the arm body.
                    // For generic enums, instantiate type params with fresh type vars.
                    let mut typed_bindings: Vec<(String, InferType)> =
                        Vec::with_capacity(bindings.len());
                    for (i, name) in bindings.iter().enumerate() {
                        let ty = if i < variant_def.data.len() {
                            if is_generic {
                                self.instantiate_enum_type_param(
                                    &variant_def.data[i],
                                    &enum_def.type_params,
                                    &mut type_param_mapping,
                                )
                            } else {
                                variant_def.data[i].clone()
                            }
                        } else {
                            InferType::Dynamic
                        };
                        typed_bindings.push((name.clone(), ty));
                    }

                    // Push scope for bindings
                    self.env.push_scope();
                    for (name, ty) in &typed_bindings {
                        self.env.define_local(name.clone(), ty.clone());
                    }

                    let typed_body = self.infer_expr(&arm.body);

                    self.env.pop_scope();

                    // Unify arm body type with result type
                    self.constraints.push(Constraint::equal(
                        typed_body.ty.clone(),
                        result_type.clone(),
                        arm.body.span,
                        ConstraintReason::Other("match arm body".to_string()),
                    ));

                    typed_arms.push(TypedMatchArm {
                        pattern: TypedPattern::Variant {
                            enum_name: pat_enum.clone(),
                            variant: variant.clone(),
                            tag: variant_def.tag,
                            bindings: typed_bindings,
                        },
                        body: Box::new(typed_body),
                    });
                }
                Pattern::Wildcard(pat_span) => {
                    if has_wildcard {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Dynamic,
                                found: InferType::Dynamic,
                            },
                            span: *pat_span,
                            reason: ConstraintReason::Other(
                                "duplicate wildcard pattern".to_string(),
                            ),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                    }
                    has_wildcard = true;

                    let typed_body = self.infer_expr(&arm.body);

                    self.constraints.push(Constraint::equal(
                        typed_body.ty.clone(),
                        result_type.clone(),
                        arm.body.span,
                        ConstraintReason::Other("match arm body".to_string()),
                    ));

                    typed_arms.push(TypedMatchArm {
                        pattern: TypedPattern::Wildcard,
                        body: Box::new(typed_body),
                    });
                }
            }
        }

        // Exhaustiveness check: all variants must be covered, OR wildcard must be present
        if !has_wildcard {
            let all_variants: HashSet<String> =
                enum_def.variants.iter().map(|v| v.name.clone()).collect();
            let missing: Vec<_> = all_variants
                .difference(&covered_variants)
                .cloned()
                .collect();
            if !missing.is_empty() {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::Mismatch {
                        expected: InferType::Enum(enum_name.clone()),
                        found: InferType::Dynamic,
                    },
                    span,
                    reason: ConstraintReason::Other(format!(
                        "non-exhaustive match: missing variant{} {}",
                        if missing.len() == 1 { "" } else { "s" },
                        missing
                            .iter()
                            .map(|v| format!("'{}::{}'", enum_name, v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
            }
        }

        (
            TypedExprKind::Match {
                scrutinee: Box::new(typed_scrutinee),
                arms: typed_arms,
            },
            result_type,
        )
    }

    /// Infer match arms when the scrutinee type is Dynamic (error recovery).
    fn infer_match_arms_dynamic(&mut self, arms: &[MatchArm]) -> Vec<TypedMatchArm> {
        arms.iter()
            .map(|arm| {
                let typed_body = self.infer_expr(&arm.body);
                let pattern = match &arm.pattern {
                    Pattern::Variant {
                        enum_name,
                        variant,
                        bindings,
                        ..
                    } => TypedPattern::Variant {
                        enum_name: enum_name.clone(),
                        variant: variant.clone(),
                        tag: 0,
                        bindings: bindings
                            .iter()
                            .map(|name| (name.clone(), InferType::Dynamic))
                            .collect(),
                    },
                    Pattern::Wildcard(_) => TypedPattern::Wildcard,
                };
                TypedMatchArm {
                    pattern,
                    body: Box::new(typed_body),
                }
            })
            .collect()
    }
}
