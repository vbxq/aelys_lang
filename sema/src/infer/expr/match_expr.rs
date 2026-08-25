use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedMatchArm, TypedPattern};
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
        let is_catch = std::mem::take(&mut self.catch_match_pending);
        let typed_scrutinee = self.infer_expr(scrutinee);
        self.infer_match_typed(typed_scrutinee, scrutinee.span, arms, span, is_catch)
    }

    pub(super) fn infer_match_typed(
        &mut self,
        typed_scrutinee: TypedExpr,
        scrutinee_span: Span,
        arms: &[MatchArm],
        span: Span,
        is_catch: bool,
    ) -> (TypedExprKind, InferType) {
        let (enum_name, scrutinee_type_args) = match &typed_scrutinee.ty {
            InferType::Enum(name, args) => (name.clone(), args.clone()),
            InferType::Var(_) => {
                let first_enum = arms.iter().find_map(|arm| match &arm.pattern {
                    Pattern::Variant { enum_name, .. } => Some(enum_name.clone()),
                    _ => None,
                });
                match first_enum {
                    Some(name) => {
                        self.constraints.push(Constraint::equal(
                            typed_scrutinee.ty.clone(),
                            InferType::Enum(name.clone(), Vec::new()),
                            scrutinee_span,
                            ConstraintReason::Other(
                                "match scrutinee inferred as enum from patterns".to_string(),
                            ),
                        ));
                        (name, Vec::new())
                    }
                    None => {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Dynamic,
                                found: typed_scrutinee.ty.clone(),
                            },
                            span: scrutinee_span,
                            reason: ConstraintReason::Other(
                                "match scrutinee type is ambiguous and no variant patterns to infer from".to_string(),
                            ),
                            secondary_spans: Vec::new(),
                            help: None,
                            suggestion: None,
                        });
                        return (TypedExprKind::Null, InferType::Dynamic);
                    }
                }
            }
            InferType::Dynamic => {
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
                    span: scrutinee_span,
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

        let mut result_type = self.type_gen.fresh();

        let is_generic = !enum_def.type_params.is_empty();
        let mut type_param_mapping: HashMap<String, InferType> = HashMap::new();

        if is_generic && scrutinee_type_args.len() == enum_def.type_params.len() {
            for (param_name, arg_ty) in enum_def.type_params.iter().zip(scrutinee_type_args.iter())
            {
                type_param_mapping.insert(param_name.clone(), arg_ty.clone());
            }
        }

        for (arm_index, arm) in arms.iter().enumerate() {
            match &arm.pattern {
                Pattern::Variant {
                    enum_name: pat_enum,
                    variant,
                    bindings,
                    span: pat_span,
                } => {
                    if *pat_enum != enum_name {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Enum(enum_name.clone(), Vec::new()),
                                found: InferType::Enum(pat_enum.clone(), Vec::new()),
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

                    let variant_def = enum_def.variants.iter().find(|v| v.name == *variant);
                    let variant_def = match variant_def {
                        Some(v) => v,
                        None => {
                            let variant_names: Vec<_> =
                                enum_def.variants.iter().map(|v| v.name.as_str()).collect();
                            self.errors.push(TypeError {
                                kind: TypeErrorKind::Mismatch {
                                    expected: InferType::Enum(enum_name.clone(), Vec::new()),
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

                    if covered_variants.contains(variant) {
                        self.errors.push(TypeError {
                            kind: TypeErrorKind::Mismatch {
                                expected: InferType::Enum(enum_name.clone(), Vec::new()),
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

                    self.env.push_scope();
                    for (name, ty) in &typed_bindings {
                        self.env.define_local(name.clone(), ty.clone());
                    }

                    let typed_body = self.infer_expr(&arm.body);

                    self.env.pop_scope();


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
                    if arm_index + 1 != arms.len() {
                        self.errors.push(TypeError::member_access(
                            "wildcard pattern must be the last match arm".to_string(),
                            *pat_span,
                        ));
                    }
                    has_wildcard = true;

                    let typed_body = self.infer_expr(&arm.body);

                    typed_arms.push(TypedMatchArm {
                        pattern: TypedPattern::Wildcard,
                        body: Box::new(typed_body),
                    });
                }
            }
        }

        // have i64 literals, narrow the literals to match. this avoids stale
        if typed_arms.len() > 1 {
            let concrete_int = typed_arms
                .iter()
                .map(|a| &a.body.ty)
                .find(|t| t.is_integer() && **t != InferType::I64)
                .cloned();
            let concrete_float = typed_arms
                .iter()
                .map(|a| &a.body.ty)
                .find(|t| t.is_float() && **t != InferType::F64)
                .cloned();
            if let Some(ref target) = concrete_int {
                for arm in &mut typed_arms {
                    if arm.body.ty == InferType::I64 {
                        self.try_narrow_literal(&mut arm.body, target);
                    }
                }
            }
            if let Some(ref target) = concrete_float {
                for arm in &mut typed_arms {
                    if arm.body.ty == InferType::F64 {
                        self.try_narrow_literal(&mut arm.body, target);
                    }
                }
            }

            let concrete_enum = typed_arms.iter()
                .map(|a| &a.body.ty)
                .find(|t| matches!(t, InferType::Enum(_, args) if args.iter().all(|a| a.is_concrete())))
                .cloned();
            if let Some(ref target) = concrete_enum {
                if let InferType::Enum(target_name, _) = target {
                    for arm in &mut typed_arms {
                        if let InferType::Enum(arm_name, arm_args) = &arm.body.ty {
                            if arm_name == target_name
                                && arm_args.iter().any(|a| matches!(a, InferType::Var(_)))
                            {
                                self.try_narrow_literal(&mut arm.body, target);
                            }
                        }
                    }
                }
            }
        }

        if typed_arms.len() > 1 {
            let widest = typed_arms
                .iter()
                .map(|a| &a.body.ty)
                .find(|t| {
                    (t.is_integer() || t.is_float())
                        && typed_arms.iter().all(|other| {
                            other.body.ty == **t
                                || other.body.ty.can_implicit_widen_to(t)
                                || !other.body.ty.is_integer() && !other.body.ty.is_float()
                        })
                })
                .cloned();
            if let Some(ref target) = widest {
                for arm in &mut typed_arms {
                    if arm.body.ty != *target && arm.body.ty.can_implicit_widen_to(target) {
                        let vspan = arm.body.span;
                        let old_body = std::mem::replace(
                            &mut arm.body,
                            Box::new(TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span: vspan,
                            }),
                        );
                        arm.body = Box::new(TypedExpr {
                            kind: TypedExprKind::Cast {
                                expr: old_body,
                                target: target.clone(),
                            },
                            ty: target.clone(),
                            span: vspan,
                        });
                    }
                }
            }
        }

        if !typed_arms.is_empty() {
            let first_ty = &typed_arms[0].body.ty;
            let all_same_concrete =
                first_ty.is_concrete() && typed_arms.iter().all(|a| a.body.ty == *first_ty);
            if all_same_concrete {
                result_type = first_ty.clone();
            }
        }

        for arm in &typed_arms {
            self.constraints.push(Constraint::flows(
                arm.body.ty.clone(),
                result_type.clone(),
                arm.body.span,
                ConstraintReason::Other("match arm body".to_string()),
            ));
        }

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
                        expected: InferType::Enum(enum_name.clone(), Vec::new()),
                        found: InferType::Dynamic,
                    },
                    span,
                    reason: ConstraintReason::Other(format!(
                        "non-exhaustive {}: missing variant{} {}",
                        if is_catch { "catch" } else { "match" },
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
