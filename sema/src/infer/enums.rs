use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedMatchArm, TypedPattern, TypedPatternKind};
use crate::types::{EnumDef, EnumVariant, InferType};
use aelys_syntax::{AttrArg, Decorator, EnumVariantDecl, Expr, MatchOrigin, PatLit, Span, Stmt, StmtKind};

impl TypeInference {
    pub(super) fn collect_enums(&mut self, stmts: &[Stmt]) {
        let mut pending = std::collections::HashSet::new();
        for stmt in stmts {
            if let StmtKind::EnumDecl {
                name, type_params, ..
            } = &stmt.kind
                && !self.type_table.has_enum(name)
            {
                pending.insert(name.clone());
                self.type_table.register_enum(EnumDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: Vec::new(),
                });
            }
        }
        for stmt in stmts {
            if let StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                repr,
                ..
            } = &stmt.kind
            {
                if !pending.remove(name) {
                    continue;
                }

                // set type params in scope so generic enum fields like `t` are recognized
                let saved_type_params =
                    std::mem::replace(&mut self.type_params_in_scope, type_params.clone());

                let mut enum_variants: Vec<EnumVariant> = variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let data = v
                            .fields
                            .iter()
                            .map(|ann| {
                                // an enum may carry an rc payload directly, but not one
                                let ty = self.type_from_annotation(ann);
                                if Self::is_transparent_aggregate_of_rc(&ty) {
                                    self.errors.push(
                                        crate::constraint::TypeError::rc_out_of_surface(format!(
                                            "variant `{}` of enum `{}` has a payload of type `{}`, \
                                             a transparent aggregate embedding an `Rc<T>`; an Rc \
                                             inside an array/vec/tuple payload is not supported yet",
                                            v.name, name, ty
                                        ), ann.span),
                                    );
                                }
                                ty
                            })
                            .collect();
                        EnumVariant {
                            name: v.name.clone(),
                            tag: i as u32,
                            data,
                            discr: i as i64,
                        }
                    })
                    .collect();

                self.type_params_in_scope = saved_type_params;

                self.assign_discriminants(name, variants, repr.as_ref(), &mut enum_variants);

                self.type_table.register_enum(EnumDef {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: enum_variants,
                });
            }
        }
    }

    fn assign_discriminants(
        &mut self,
        name: &str,
        decls: &[EnumVariantDecl],
        repr: Option<&Decorator>,
        variants: &mut [EnumVariant],
    ) {
        if repr.is_none() && decls.iter().all(|v| v.discriminant.is_none()) {
            return;
        }
        if let Some(carrier) = decls.iter().find(|v| !v.fields.is_empty()) {
            let what = if repr.is_some() {
                "`#[repr]`"
            } else {
                "an explicit discriminant"
            };
            self.errors.push(TypeError::coded(
                "E0454",
                format!(
                    "{what} needs an enum whose variants carry no data, and `{name}::{}` does",
                    carrier.name
                ),
                "variant with data",
                carrier.span,
            ));
            return;
        }
        let ty = match repr {
            None => InferType::I64,
            Some(d) => {
                let arg = match d.args.as_slice() {
                    [AttrArg::Word(w)] => w.as_str(),
                    _ => "",
                };
                let ty = InferType::from_name(arg);
                if !ty.is_integer() {
                    self.errors.push(TypeError::coded(
                        "E0454",
                        format!("unrecognized representation hint `{arg}`; `#[repr]` takes an integer type"),
                        "not an integer type",
                        d.span,
                    ));
                    return;
                }
                ty
            }
        };
        let Some((lo, hi)) = ty.int_bounds() else {
            return;
        };
        let eval_ty = if ty == InferType::U64 {
            InferType::U64
        } else {
            InferType::I64
        };
        let mut seen: Vec<(i128, &str)> = Vec::new();
        let mut next: i128 = 0;
        for (decl, variant) in decls.iter().zip(variants.iter_mut()) {
            let value = match &decl.discriminant {
                Some(e) => {
                    let owner = format!("{name}::{}", decl.name);
                    let Some(v) = self.eval_discriminant(&owner, e, &eval_ty) else {
                        return;
                    };
                    if v < lo || v > hi {
                        self.errors.push(TypeError::coded(
                            "E0453",
                            format!("discriminant value `{v}` does not fit in `{ty}`"),
                            "out of range",
                            e.span,
                        ));
                        return;
                    }
                    v
                }
                None if next > hi => {
                    self.errors.push(TypeError::coded(
                        "E0453",
                        format!("enum discriminant overflowed: `{name}::{}` would be {next}, which does not fit in `{ty}`", decl.name),
                        "overflowed",
                        decl.span,
                    ));
                    return;
                }
                None => next,
            };
            if let Some((_, first)) = seen.iter().find(|(v, _)| *v == value) {
                self.errors.push(TypeError::coded(
                    "E0452",
                    format!("discriminant value `{value}` assigned more than once, to `{name}::{first}` and `{name}::{}`", decl.name),
                    "duplicate discriminant",
                    decl.span,
                ));
                return;
            }
            seen.push((value, &decl.name));
            variant.discr = value as i64;
            next = value + 1;
        }
    }

    pub(super) fn infer_enum_from_i64(
        &mut self,
        def: &EnumDef,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let shown = crate::modules::source_type_name(&def.name);
        let [arg] = args else {
            self.errors.push(TypeError {
                kind: TypeErrorKind::ArityMismatch {
                    expected: 1,
                    found: args.len(),
                },
                span,
                reason: ConstraintReason::Other(format!("`{shown}::from_i64` takes one `i64`")),
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        let mut n = self.infer_expr(arg);
        self.try_narrow_literal(&mut n, &InferType::I64);
        self.constraints.push(Constraint::equal(
            n.ty.clone(),
            InferType::I64,
            arg.span,
            ConstraintReason::Other(format!("argument of `{shown}::from_i64`")),
        ));
        let enum_ty = InferType::Enum(def.name.clone(), Vec::new());
        let result = self.option_of(enum_ty.clone(), span);
        let InferType::Enum(option, _) = &result else {
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        let tag_of = |name: &str| {
            self.type_table
                .get_enum(option)
                .and_then(|d| d.variants.iter().find(|v| v.name == name))
                .map_or(0, |v| v.tag)
        };
        let (some_tag, none_tag) = (tag_of("Some"), tag_of("None"));
        let leaf = |kind: TypedExprKind, ty: &InferType| TypedExpr {
            kind,
            ty: ty.clone(),
            span,
        };
        let mut arms: Vec<TypedMatchArm> = def
            .variants
            .iter()
            .map(|v| {
                let value = leaf(
                    TypedExprKind::EnumVariant {
                        enum_name: def.name.clone(),
                        variant: v.name.clone(),
                        tag: v.tag,
                        args: Vec::new(),
                    },
                    &enum_ty,
                );
                TypedMatchArm::new(
                    TypedPattern::new(
                        TypedPatternKind::Lit(PatLit::Int(v.discr as i128)),
                        InferType::I64,
                        span,
                    ),
                    None,
                    leaf(
                        TypedExprKind::EnumVariant {
                            enum_name: option.clone(),
                            variant: "Some".to_string(),
                            tag: some_tag,
                            args: vec![value],
                        },
                        &result,
                    ),
                )
            })
            .collect();
        arms.push(TypedMatchArm::new(
            TypedPattern::new(TypedPatternKind::Wildcard, InferType::I64, span),
            None,
            leaf(
                TypedExprKind::EnumVariant {
                    enum_name: option.clone(),
                    variant: "None".to_string(),
                    tag: none_tag,
                    args: Vec::new(),
                },
                &result,
            ),
        ));
        (
            TypedExprKind::Match {
                scrutinee: Box::new(n),
                arms,
                origin: MatchOrigin::Match,
            },
            result,
        )
    }
}
