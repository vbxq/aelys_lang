use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::modules::source_type_name;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedMatchArm, TypedPattern, TypedPatternKind};
use crate::types::InferType;
use aelys_syntax::{Expr, MatchArm, MatchOrigin, PatLit, Pattern, Span};
use std::collections::HashMap;

impl TypeInference {
    pub(super) fn infer_match_expr(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        origin: MatchOrigin,
        expect: Option<&InferType>,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_scrutinee = self.infer_expr(scrutinee);
        self.infer_match_typed(typed_scrutinee, scrutinee.span, arms, span, origin, expect)
    }

    pub(super) fn infer_match_typed(
        &mut self,
        typed_scrutinee: TypedExpr,
        scrutinee_span: Span,
        arms: &[MatchArm],
        span: Span,
        origin: MatchOrigin,
        expect: Option<&InferType>,
    ) -> (TypedExprKind, InferType) {
        let typed_scrutinee = match &typed_scrutinee.ty {
            InferType::Ref { referent, .. } => {
                let ty = (**referent).clone();
                let span = typed_scrutinee.span;
                TypedExpr::new(TypedExprKind::Deref(Box::new(typed_scrutinee)), ty, span)
            }
            _ => typed_scrutinee,
        };
        let mut expected = typed_scrutinee.ty.clone();
        if let InferType::Var(_) = expected
            && let Some(fixed) = self.scrutinee_type_from_patterns(arms)
        {
            self.constraints.push(Constraint::equal(
                expected.clone(),
                fixed.clone(),
                scrutinee_span,
                ConstraintReason::Other("match scrutinee inferred from patterns".to_string()),
            ));
            expected = fixed;
        }
        if let InferType::Dynamic = expected {
            let typed_arms = self.infer_match_arms_dynamic(arms);
            let result_type = typed_arms
                .first()
                .map_or(InferType::Dynamic, |a| a.body().ty.clone());
            return (
                TypedExprKind::Match {
                    scrutinee: Box::new(typed_scrutinee),
                    arms: typed_arms,
                    origin,
                },
                result_type,
            );
        }

        let mut typed_arms: Vec<TypedMatchArm> = Vec::new();
        let mut arm_refused = false;
        let mut result_type = self.type_gen.fresh();

        for arm in arms {
            let before = self.errors.len();
            let mut seen = Vec::new();
            let pattern = self.check_pattern(&arm.pattern, &expected, &mut seen);
            let Some(pattern) = pattern.filter(|_| self.errors.len() == before) else {
                arm_refused = true;
                continue;
            };
            self.env.push_scope();
            for (name, ty, _) in pattern.bindings() {
                self.env.define_local(name.to_string(), ty.clone());
            }
            let guard = arm.guard.as_ref().map(|g| {
                let typed = self.infer_expr(g);
                self.constraints.push(Constraint::equal(
                    typed.ty.clone(),
                    InferType::Bool,
                    g.span,
                    ConstraintReason::IfCondition,
                ));
                typed
            });
            self.dyn_expect = expect.cloned();
            let typed_body = self.infer_expr(&arm.body);
            self.env.pop_scope();
            typed_arms.push(TypedMatchArm::new(pattern, guard, typed_body));
        }
        if arm_refused {
            self.refused_matches.insert((span.start, span.end));
        }

        // have i64 literals, narrow the literals to match. this avoids stale
        if typed_arms.len() > 1 {
            let fixed: Vec<InferType> = typed_arms
                .iter()
                .filter(|a| !self.is_literal_like(a.body()))
                .map(|a| a.body().ty.clone())
                .filter(|t| t.is_integer() || t.is_float())
                .collect();
            let join = fixed
                .iter()
                .find(|t| {
                    fixed
                        .iter()
                        .all(|other| other == *t || other.can_implicit_widen_to(t))
                })
                .cloned();
            if let Some(ref target) = join {
                for arm in &mut typed_arms {
                    let literal_arm = self.is_literal_like(arm.body());
                    if literal_arm
                        && (arm.body().ty == InferType::I64 || arm.body().ty == InferType::F64)
                    {
                        self.try_narrow_literal(arm.body_mut(), target);
                    }
                }
            }

            let concrete_enum = typed_arms.iter()
                .map(|a| &a.body().ty)
                .find(|t| matches!(t, InferType::Enum(_, args) if args.iter().all(|a| a.is_concrete())))
                .cloned();
            if let Some(ref target) = concrete_enum {
                if let InferType::Enum(target_name, _) = target {
                    for arm in &mut typed_arms {
                        if let InferType::Enum(arm_name, arm_args) = &arm.body().ty {
                            if arm_name == target_name
                                && arm_args.iter().any(|a| matches!(a, InferType::Var(_)))
                            {
                                self.try_narrow_literal(arm.body_mut(), target);
                            }
                        }
                    }
                }
            }
        }

        if typed_arms.len() > 1 {
            let widest = typed_arms
                .iter()
                .map(|a| &a.body().ty)
                .find(|t| {
                    (t.is_integer() || t.is_float())
                        && typed_arms.iter().all(|other| {
                            other.body().ty == **t
                                || other.body().ty.can_implicit_widen_to(t)
                                || !other.body().ty.is_integer() && !other.body().ty.is_float()
                        })
                })
                .cloned();
            if let Some(ref target) = widest {
                for arm in &mut typed_arms {
                    if arm.body().ty != *target && arm.body().ty.can_implicit_widen_to(target) {
                        let vspan = arm.body().span;
                        let old_body = std::mem::replace(
                            arm.body_box_mut(),
                            Box::new(TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span: vspan,
                            }),
                        );
                        *arm.body_box_mut() = Box::new(TypedExpr {
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
            let first_ty = &typed_arms[0].body().ty;
            let all_same_concrete =
                first_ty.is_concrete() && typed_arms.iter().all(|a| a.body().ty == *first_ty);
            if all_same_concrete {
                result_type = first_ty.clone();
            }
        }

        for arm in &typed_arms {
            self.constraints.push(Constraint::flows(
                arm.body().ty.clone(),
                result_type.clone(),
                arm.body().span,
                ConstraintReason::Other("match arm body".to_string()),
            ));
        }

        (
            TypedExprKind::Match {
                scrutinee: Box::new(typed_scrutinee),
                arms: typed_arms,
                origin,
            },
            result_type,
        )
    }

    pub(crate) fn match_was_refused(&self, span: Span) -> bool {
        self.refused_matches.contains(&(span.start, span.end))
    }

    fn scrutinee_type_from_patterns(&mut self, arms: &[MatchArm]) -> Option<InferType> {
        fn first_informative(p: &Pattern) -> Option<&Pattern> {
            match p {
                Pattern::Wildcard(_) => None,
                Pattern::Binding { sub, .. } => sub.as_deref().and_then(first_informative),
                Pattern::Or(alts, _) => alts.iter().find_map(first_informative),
                Pattern::Lit(PatLit::Int(_), _) | Pattern::Range { .. } => None,
                other => Some(other),
            }
        }
        let pat = arms.iter().find_map(|a| first_informative(&a.pattern))?;
        let converted = self.struct_pattern(pat);
        match converted.as_ref().unwrap_or(pat) {
            Pattern::Variant {
                enum_name, span, ..
            } if !enum_name.is_empty() => Some(InferType::Enum(
                self.air_type_name(enum_name, *span),
                Vec::new(),
            )),
            Pattern::Struct { name, span, .. } => {
                let name = self.air_type_name(name, *span);
                let params = self
                    .type_table
                    .get_struct(&name)
                    .map_or(0, |d| d.type_params.len());
                let args = (0..params).map(|_| self.type_gen.fresh()).collect();
                Some(InferType::Struct(name, args))
            }
            Pattern::Lit(PatLit::Char(_), _) => Some(InferType::Char),
            Pattern::Lit(PatLit::Bool(_), _) => Some(InferType::Bool),
            Pattern::Lit(PatLit::Str(_), _) => Some(InferType::StrRef),
            _ => None,
        }
    }

    fn pattern_bound_literal(&mut self, bound: &Pattern) -> Option<PatLit> {
        match bound {
            Pattern::Lit(lit, _) => Some(lit.clone()),
            Pattern::Variant {
                enum_name,
                variant,
                fields,
                unqualified: true,
                ..
            } if enum_name.is_empty() && fields.is_empty() => self.const_pattern_literal(variant),
            Pattern::Variant {
                enum_name,
                variant,
                fields,
                unqualified: false,
                span,
            } if fields.is_empty() && self.is_module_namespace(enum_name) => {
                let (qualified, _) = self.resolve_module_member(enum_name, variant, *span);
                self.const_pattern_literal(&qualified)
            }
            _ => None,
        }
    }

    fn const_pattern_literal(&self, name: &str) -> Option<PatLit> {
        use crate::typed_ast::ConstScalar;
        match &self.consts.get(name)?.value {
            ConstScalar::Int(v) => Some(PatLit::Int(*v as i128)),
            ConstScalar::Bool(b) => Some(PatLit::Bool(*b)),
            ConstScalar::Char(c) => Some(PatLit::Char(*c)),
            ConstScalar::Str(s) => Some(PatLit::Str(s.clone())),
            ConstScalar::Float(_) => None,
        }
    }

    pub(crate) fn pattern_error(&mut self, kind: TypeErrorKind, span: Span) {
        self.errors.push(TypeError::pattern(kind, span));
    }

    fn pattern_mismatch(&mut self, reason: String, span: Span) {
        self.errors.push(TypeError {
            kind: TypeErrorKind::Mismatch {
                expected: InferType::Dynamic,
                found: InferType::Dynamic,
            },
            span,
            reason: ConstraintReason::Other(reason),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        });
    }

    fn check_pattern(
        &mut self,
        pat: &Pattern,
        expected: &InferType,
        seen: &mut Vec<String>,
    ) -> Option<TypedPattern> {
        if let Some(converted) = self.struct_pattern(pat) {
            return self.check_pattern(&converted, expected, seen);
        }
        let span = pat.span();
        match pat {
            Pattern::Wildcard(_) => Some(TypedPattern::new(
                TypedPatternKind::Wildcard,
                expected.clone(),
                span,
            )),
            Pattern::Binding {
                name, sub, mutable, ..
            } => {
                if *mutable {
                    self.pattern_error(
                        TypeErrorKind::PatternUnsupported {
                            detail: "a `mut` binding is not supported in a match pattern; bind \
                                     the value, then write `let mut` in the arm"
                                .to_string(),
                        },
                        span,
                    );
                    return None;
                }
                if seen.contains(name) {
                    self.errors.push(TypeError::coded(
                        "E0204",
                        format!("`{name}` is bound more than once in the same pattern"),
                        "bound again here",
                        span,
                    ));
                    return None;
                }
                seen.push(name.clone());
                let sub = match sub {
                    Some(sub) => Some(Box::new(self.check_pattern(sub, expected, seen)?)),
                    None => None,
                };
                Some(TypedPattern::new(
                    TypedPatternKind::Binding {
                        name: name.clone(),
                        sub,
                    },
                    expected.clone(),
                    span,
                ))
            }
            Pattern::Lit(lit, _) => {
                self.check_pattern_literal(lit, expected, span)?;
                Some(TypedPattern::new(
                    TypedPatternKind::Lit(lit.clone()),
                    expected.clone(),
                    span,
                ))
            }
            Pattern::ConstRange { lo, hi, .. } => {
                let mut bounds = Vec::with_capacity(2);
                for bound in [lo.as_ref(), hi.as_ref()] {
                    let Some(lit) = self.pattern_bound_literal(bound) else {
                        self.pattern_error(
                            TypeErrorKind::PatternUnsupported {
                                detail: "a range bound is an integer or `char` literal or a \
                                         const of that type"
                                    .to_string(),
                            },
                            bound.span(),
                        );
                        return None;
                    };
                    bounds.push(lit);
                }
                let hi = bounds.pop()?;
                let lo = bounds.pop()?;
                let empty = match (&lo, &hi) {
                    (PatLit::Int(a), PatLit::Int(b)) => a > b,
                    (PatLit::Char(a), PatLit::Char(b)) => a > b,
                    _ => false,
                };
                if empty {
                    self.pattern_error(
                        TypeErrorKind::PatternUnsupported {
                            detail: "empty range pattern: no value lies between these bounds; \
                                     write the smaller bound first"
                                .to_string(),
                        },
                        span,
                    );
                    return None;
                }
                let resolved = Pattern::Range { lo, hi, span };
                self.check_pattern(&resolved, expected, seen)
            }
            Pattern::Range { lo, hi, .. } => {
                let same_kind = matches!(
                    (lo, hi),
                    (PatLit::Int(_), PatLit::Int(_)) | (PatLit::Char(_), PatLit::Char(_))
                );
                if !same_kind {
                    self.pattern_error(
                        TypeErrorKind::PatternUnsupported {
                            detail: "a range pattern takes two integer bounds or two `char` \
                                     bounds"
                                .to_string(),
                        },
                        span,
                    );
                    return None;
                }
                self.check_pattern_literal(lo, expected, span)?;
                self.check_pattern_literal(hi, expected, span)?;
                Some(TypedPattern::new(
                    TypedPatternKind::Range {
                        lo: lo.clone(),
                        hi: hi.clone(),
                    },
                    expected.clone(),
                    span,
                ))
            }
            Pattern::Variant {
                enum_name,
                variant,
                fields,
                unqualified,
                ..
            } => self.check_variant_pattern(
                enum_name,
                variant,
                fields,
                *unqualified,
                expected,
                span,
                seen,
            ),
            Pattern::Struct {
                name, fields, rest, ..
            } => self.check_struct_pattern(name, fields, *rest, expected, span, seen),
            Pattern::Tuple { elems, .. } => {
                let InferType::Tuple(tys) = expected else {
                    self.pattern_mismatch(
                        format!(
                            "a tuple pattern cannot match a value of type `{}`",
                            source_type_name(&expected.to_string())
                        ),
                        span,
                    );
                    return None;
                };
                if tys.len() != elems.len() {
                    self.pattern_mismatch(
                        format!(
                            "this tuple pattern has {} elements but the value is a tuple of {}",
                            elems.len(),
                            tys.len()
                        ),
                        span,
                    );
                    return None;
                }
                let tys = tys.clone();
                let mut typed = Vec::with_capacity(elems.len());
                for (e, t) in elems.iter().zip(tys.iter()) {
                    typed.push(self.check_pattern(e, t, seen)?);
                }
                Some(TypedPattern::new(
                    TypedPatternKind::Tuple(typed),
                    expected.clone(),
                    span,
                ))
            }
            Pattern::Or(alts, _) => self.check_or_pattern(alts, expected, span, seen),
        }
    }

    fn check_or_pattern(
        &mut self,
        alts: &[Pattern],
        expected: &InferType,
        span: Span,
        seen: &mut Vec<String>,
    ) -> Option<TypedPattern> {
        let mut typed = Vec::with_capacity(alts.len());
        for alt in alts {
            let mut alt_seen = seen.clone();
            typed.push(self.check_pattern(alt, expected, &mut alt_seen)?);
        }
        let first: Vec<(String, InferType, Span)> = typed[0]
            .bindings()
            .into_iter()
            .map(|(n, t, s)| (n.to_string(), t.clone(), s))
            .collect();
        for alt in typed.iter().skip(1) {
            let other: Vec<(String, InferType, Span)> = alt
                .bindings()
                .into_iter()
                .map(|(n, t, s)| (n.to_string(), t.clone(), s))
                .collect();
            let mut differ: Vec<(String, Span)> = first
                .iter()
                .filter(|(n, _, _)| !other.iter().any(|(m, _, _)| m == n))
                .chain(
                    other
                        .iter()
                        .filter(|(n, _, _)| !first.iter().any(|(m, _, _)| m == n)),
                )
                .map(|(n, _, s)| (n.clone(), *s))
                .collect();
            if !differ.is_empty() {
                let at = differ[0].1;
                let names: Vec<String> = differ.drain(..).map(|(n, _)| format!("`{n}`")).collect();
                self.pattern_error(
                    TypeErrorKind::PatternAltMismatch {
                        detail: format!(
                            "the alternatives of this pattern bind different names: {} {} not \
                             bound in every alternative; every alternative must bind the same \
                             names",
                            names.join(" and "),
                            if names.len() == 1 { "is" } else { "are" }
                        ),
                    },
                    at,
                );
                return None;
            }
            for (name, ty, at) in &other {
                let (_, first_ty, _) = first.iter().find(|(n, _, _)| n == name)?;
                if first_ty == ty {
                    continue;
                }
                let open = |t: &InferType| matches!(t, InferType::Var(_)) || !t.is_concrete();
                if (open(first_ty) || open(ty))
                    && !matches!(first_ty, InferType::Never)
                    && !matches!(ty, InferType::Never)
                {
                    self.constraints.push(Constraint::equal(
                        first_ty.clone(),
                        ty.clone(),
                        *at,
                        ConstraintReason::Other("binding of a pattern alternative".to_string()),
                    ));
                    continue;
                }
                self.pattern_error(
                    TypeErrorKind::PatternAltMismatch {
                        detail: format!(
                            "`{name}` has type `{}` in one alternative of this pattern and `{}` \
                             in another; every alternative must bind it with the same type",
                            source_type_name(&first_ty.to_string()),
                            source_type_name(&ty.to_string())
                        ),
                    },
                    *at,
                );
                return None;
            }
        }
        seen.extend(first.into_iter().map(|(n, _, _)| n));
        Some(TypedPattern::new(
            TypedPatternKind::Or(typed),
            expected.clone(),
            span,
        ))
    }

    fn check_pattern_literal(
        &mut self,
        lit: &PatLit,
        expected: &InferType,
        span: Span,
    ) -> Option<()> {
        let ok = match (lit, expected) {
            (PatLit::Int(v), t) if t.is_integer() => {
                let Some((lo, hi)) = t.int_bounds() else {
                    self.pattern_mismatch(
                        format!("an integer pattern cannot match a `{t}` yet"),
                        span,
                    );
                    return None;
                };
                if *v < lo || *v > hi {
                    self.pattern_mismatch(
                        format!("pattern `{v}` does not fit `{t}`, whose values are {lo}..={hi}"),
                        span,
                    );
                    return None;
                }
                true
            }
            (PatLit::Int(_), InferType::Var(_)) => true,
            (PatLit::Char(_), InferType::Char)
            | (PatLit::Bool(_), InferType::Bool)
            | (PatLit::Str(_), InferType::String | InferType::StrRef) => true,
            _ => false,
        };
        if !ok {
            let lit_ty = match lit {
                PatLit::Int(_) => "an integer",
                PatLit::Char(_) => "`char`",
                PatLit::Bool(_) => "`bool`",
                PatLit::Str(_) => "`string`",
            };
            self.pattern_mismatch(
                format!(
                    "this literal pattern is {lit_ty}, but the matched value has type `{}`",
                    source_type_name(&expected.to_string())
                ),
                span,
            );
            return None;
        }
        Some(())
    }

    #[allow(clippy::too_many_arguments)]
    fn check_variant_pattern(
        &mut self,
        written_enum: &str,
        variant: &str,
        fields: &[Pattern],
        unqualified: bool,
        expected: &InferType,
        span: Span,
        seen: &mut Vec<String>,
    ) -> Option<TypedPattern> {
        if !unqualified
            && fields.is_empty()
            && self.is_module_namespace(written_enum)
        {
            let module_const = Pattern::Variant {
                enum_name: written_enum.to_string(),
                variant: variant.to_string(),
                fields: Vec::new(),
                span,
                unqualified: false,
            };
            let Some(lit) = self.pattern_bound_literal(&module_const) else {
                self.errors.push(TypeError::coded(
                    "E0101",
                    format!(
                        "`{written_enum}.{variant}` names no const of the module `{written_enum}`; \
                         a module value in a pattern must be a const"
                    ),
                    "not a const",
                    span,
                ));
                return None;
            };
            self.check_pattern_literal(&lit, expected, span)?;
            return Some(TypedPattern::new(
                TypedPatternKind::Lit(lit),
                expected.clone(),
                span,
            ));
        }
        if unqualified && written_enum.is_empty() {
            if fields.is_empty()
                && let Some(lit) = self.const_pattern_literal(variant)
            {
                self.check_pattern_literal(&lit, expected, span)?;
                return Some(TypedPattern::new(
                    TypedPatternKind::Lit(lit),
                    expected.clone(),
                    span,
                ));
            }
            let InferType::Enum(enum_name, _) = expected else {
                self.errors.push(TypeError::coded(
                    "E0101",
                    format!(
                        "`{variant}` is not a variant in scope; read as a binding, the binding \
                         `{variant}` starts with an uppercase letter; a capitalized name in a \
                         pattern names a variant, never a binding"
                    ),
                    "not a variant in scope",
                    span,
                ));
                return None;
            };
            let variants: Vec<String> = self
                .type_table
                .get_enum(enum_name)
                .map(|def| def.variants.iter().map(|v| v.name.clone()).collect())
                .unwrap_or_default();
            let written = variants
                .iter()
                .any(|v| v == variant)
                .then(|| self.spelled_enum(enum_name).ok())
                .flatten();
            let owner = match written {
                Some(_) => None,
                None => self.enum_owning_variant(variant, enum_name),
            };
            let scrutinee = self
                .spelled_enum(enum_name)
                .unwrap_or_else(|_| source_type_name(enum_name).to_string());
            self.errors.push(TypeError {
                kind: TypeErrorKind::NotAVariantInScope {
                    name: variant.to_string(),
                    written,
                    scrutinee,
                    variants,
                    owner,
                },
                span,
                reason: ConstraintReason::Other(String::new()),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            return None;
        }
        if unqualified
            && let Some(refusal) = self.unqualified_variant_refusal(written_enum, variant, span)
        {
            self.errors.push(refusal);
            return None;
        }
        let pat_enum = self.air_type_name(written_enum, span);
        let (enum_name, scrutinee_args) = match expected {
            InferType::Enum(name, args) => (name.clone(), args.clone()),
            other => {
                self.pattern_mismatch(
                    format!(
                        "pattern `{variant}` is a variant of `{}`, but the matched value has type \
                         `{}`",
                        source_type_name(&pat_enum),
                        source_type_name(&other.to_string())
                    ),
                    span,
                );
                return None;
            }
        };
        if unqualified && pat_enum != enum_name {
            let scrutinee = self.spelled_enum(&enum_name);
            self.errors.push(TypeError::unqualified_pattern_mismatch(
                &pat_enum, scrutinee, variant, span,
            ));
            return None;
        }
        if pat_enum != enum_name {
            let reason = match self.spelled_enum(&enum_name) {
                Ok(path) => {
                    format!("pattern enum '{written_enum}' does not match scrutinee enum '{path}'")
                }
                Err((module, name)) => {
                    let alias = module.rsplit('.').next().unwrap_or(&module);
                    format!(
                        "pattern enum '{written_enum}' does not match the scrutinee's \
                         enum, the `{name}` of `{module}`, which no name in scope \
                         reaches; add `use {};` and write \
                         `{alias}::{name}::{variant}`",
                        module.replace('.', "::")
                    )
                }
            };
            self.errors.push(TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: InferType::Dynamic,
                    found: InferType::Enum(pat_enum.clone(), Vec::new()),
                },
                span,
                reason: ConstraintReason::Other(reason),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            return None;
        }
        let Some(enum_def) = self.type_table.get_enum(&enum_name).cloned() else {
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
            return None;
        };
        let Some(variant_def) = enum_def.variants.iter().find(|v| v.name == variant) else {
            let variant_names: Vec<_> = enum_def.variants.iter().map(|v| v.name.as_str()).collect();
            self.pattern_mismatch(
                format!(
                    "unknown variant '{}' on enum '{}'; known variants: {}",
                    variant,
                    source_type_name(&enum_name),
                    variant_names.join(", ")
                ),
                span,
            );
            return None;
        };
        let expected_fields = variant_def.data.len();
        if expected_fields != fields.len() {
            self.errors.push(TypeError {
                kind: TypeErrorKind::ArityMismatch {
                    expected: expected_fields,
                    found: fields.len(),
                },
                span,
                reason: ConstraintReason::Other(format!(
                    "variant '{}::{}' has {} data field{}, but pattern has {} binding{}",
                    source_type_name(&enum_name),
                    variant,
                    expected_fields,
                    if expected_fields == 1 { "" } else { "s" },
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                )),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            return None;
        }
        let is_generic = !enum_def.type_params.is_empty();
        let mut mapping: HashMap<String, InferType> = HashMap::new();
        if is_generic && scrutinee_args.len() == enum_def.type_params.len() {
            for (param, arg) in enum_def.type_params.iter().zip(scrutinee_args.iter()) {
                mapping.insert(param.clone(), arg.clone());
            }
        }
        let mut typed_fields = Vec::with_capacity(fields.len());
        for (field, data_ty) in fields.iter().zip(variant_def.data.iter()) {
            let field_ty = if is_generic {
                self.instantiate_enum_type_param(data_ty, &enum_def.type_params, &mut mapping)
            } else {
                data_ty.clone()
            };
            typed_fields.push(self.check_pattern(field, &field_ty, seen)?);
        }
        Some(TypedPattern::new(
            TypedPatternKind::Variant {
                enum_name: pat_enum,
                variant: variant.to_string(),
                tag: variant_def.tag,
                fields: typed_fields,
            },
            expected.clone(),
            span,
        ))
    }

    fn check_struct_pattern(
        &mut self,
        written: &str,
        fields: &[(String, Pattern, Span)],
        rest: bool,
        expected: &InferType,
        span: Span,
        seen: &mut Vec<String>,
    ) -> Option<TypedPattern> {
        let name = self.air_type_name(written, span);
        let def_fields = match expected {
            InferType::Struct(s, args) if *s == name => self.type_table.struct_fields(&name, args),
            _ => None,
        };
        let Some(def_fields) = def_fields else {
            self.pattern_mismatch(
                format!(
                    "pattern `{written} {{ .. }}` matches a `{written}`, but the matched value has \
                     type `{}`",
                    source_type_name(&expected.to_string())
                ),
                span,
            );
            return None;
        };
        let mut given: Vec<(&str, &Pattern, Span)> = Vec::new();
        for (field, pat, at) in fields {
            if !def_fields.iter().any(|f| f.name == *field) {
                self.pattern_error(
                    TypeErrorKind::PatternMalformed {
                        detail: format!("struct `{written}` has no field `{field}`"),
                    },
                    *at,
                );
                return None;
            }
            if given.iter().any(|(g, _, _)| g == field) {
                self.pattern_error(
                    TypeErrorKind::PatternMalformed {
                        detail: format!("field `{field}` is named twice in this pattern"),
                    },
                    *at,
                );
                return None;
            }
            given.push((field.as_str(), pat, *at));
        }
        if !rest
            && let Some(missing) = def_fields
                .iter()
                .find(|f| !given.iter().any(|(g, _, _)| *g == f.name))
        {
            self.pattern_error(
                TypeErrorKind::PatternMalformed {
                    detail: format!(
                        "this pattern does not mention field `{}` of `{written}`; name it, or \
                         write `..` to skip the fields left out",
                        missing.name
                    ),
                },
                span,
            );
            return None;
        }
        let mut typed = Vec::with_capacity(def_fields.len());
        for f in &def_fields {
            let pat = match given.iter().find(|(g, _, _)| *g == f.name) {
                Some((_, pat, _)) => self.check_pattern(pat, &f.ty, seen)?,
                None => TypedPattern::new(TypedPatternKind::Wildcard, f.ty.clone(), span),
            };
            typed.push((f.name.clone(), pat));
        }
        Some(TypedPattern::new(
            TypedPatternKind::Struct {
                name,
                fields: typed,
            },
            expected.clone(),
            span,
        ))
    }

    fn infer_match_arms_dynamic(&mut self, arms: &[MatchArm]) -> Vec<TypedMatchArm> {
        let mut out = Vec::new();
        for arm in arms {
            let pattern = match &arm.pattern {
                Pattern::Variant {
                    enum_name,
                    variant,
                    fields,
                    span,
                    ..
                } if arm.guard.is_none()
                    && fields.iter().all(|f| {
                        matches!(f, Pattern::Wildcard(_) | Pattern::Binding { sub: None, .. })
                    }) =>
                {
                    let fields = fields
                        .iter()
                        .map(|f| match f {
                            Pattern::Binding { name, span, .. } => TypedPattern::new(
                                TypedPatternKind::Binding {
                                    name: name.clone(),
                                    sub: None,
                                },
                                InferType::Dynamic,
                                *span,
                            ),
                            other => TypedPattern::new(
                                TypedPatternKind::Wildcard,
                                InferType::Dynamic,
                                other.span(),
                            ),
                        })
                        .collect();
                    TypedPattern::new(
                        TypedPatternKind::Variant {
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            tag: 0,
                            fields,
                        },
                        InferType::Dynamic,
                        *span,
                    )
                }
                Pattern::Wildcard(span) if arm.guard.is_none() => {
                    TypedPattern::new(TypedPatternKind::Wildcard, InferType::Dynamic, *span)
                }
                other => {
                    self.pattern_mismatch(
                        "the scrutinee's type is not known, and this pattern needs it".to_string(),
                        other.span(),
                    );
                    continue;
                }
            };
            self.env.push_scope();
            for (name, ty, _) in pattern.bindings() {
                self.env.define_local(name.to_string(), ty.clone());
            }
            let typed_body = self.infer_expr(&arm.body);
            self.env.pop_scope();
            out.push(TypedMatchArm::new(pattern, None, typed_body));
        }
        out
    }
}

