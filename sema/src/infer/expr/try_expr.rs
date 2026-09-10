use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::modules::source_type_name;
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, MatchArm, Pattern, Span, Stmt, StmtKind};

const AMBIGUOUS: &str = "[?-stage1] cannot determine the type of the `?` operand here; give it an explicit `Result`/`Option` type";
const NOT_A_CARRIER: &str = "not a `Result` or `Option` value";
const NO_CARRIER_RETURN: &str = "this function does not return a `Result`/`Option`";

enum Carrier {
    Result { enum_name: String, error: InferType },
    Option { enum_name: String },
}

impl Carrier {
    fn enum_name(&self) -> &str {
        match self {
            Carrier::Result { enum_name, .. } => enum_name,
            Carrier::Option { enum_name } => enum_name,
        }
    }

    fn source_name(&self) -> &'static str {
        match self {
            Carrier::Result { .. } => "Result",
            Carrier::Option { .. } => "Option",
        }
    }
}

impl TypeInference {
    pub(super) fn infer_try_expr(
        &mut self,
        inner: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_inner = self.infer_expr(inner);

        if matches!(typed_inner.ty, InferType::Dynamic) {
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        let carrier = match classify_carrier(&typed_inner.ty) {
            Ok(c) => c,
            Err(msg) => {
                self.errors.push(TypeError::error_handling(
                    msg.to_string(),
                    NOT_A_CARRIER,
                    span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let ret_ty = match self.current_return_type().cloned() {
            Some(t) => t,
            None => {
                self.errors.push(TypeError::error_handling(
                    "[?-stage1] `?` used outside a function that returns `Result`/`Option`"
                        .to_string(),
                    NO_CARRIER_RETURN,
                    span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let ret_carrier = match &ret_ty {
            InferType::Enum(name, args)
                if source_type_name(name) == "Result" && args.len() == 2 =>
            {
                Some(("Result", name.clone()))
            }
            InferType::Enum(name, args)
                if source_type_name(name) == "Option" && args.len() == 1 =>
            {
                Some(("Option", name.clone()))
            }
            _ => None,
        };

        if let Some((ret_source, ret_name)) = &ret_carrier
            && *ret_source == carrier.source_name()
            && ret_name != carrier.enum_name()
        {
            self.errors.push(TypeError::carrier_mismatch(
                carrier.source_name(),
                typed_inner.ty.clone(),
                ret_ty.clone(),
                span,
            ));
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        match &carrier {
            Carrier::Result { error: e_op, .. } => {
                let e_fn = match (&ret_carrier, &ret_ty) {
                    (Some(("Result", _)), InferType::Enum(_, args)) => args[1].clone(),
                    _ => {
                        self.errors.push(TypeError::error_handling(
                            "[?-stage1] `?` on a `Result` requires the function to return `Result<_, E>`"
                                .to_string(),
                            NO_CARRIER_RETURN,
                            span,
                        ));
                        return (TypedExprKind::Null, InferType::Dynamic);
                    }
                };
                self.constraints.push(Constraint::equal(
                    e_op.clone(),
                    e_fn,
                    span,
                    ConstraintReason::QuestionErrorType,
                ));
            }
            Carrier::Option { .. } => {
                if !matches!(&ret_carrier, Some(("Option", _))) {
                    self.errors.push(TypeError::error_handling(
                        "[?-stage1] `?` on an `Option` requires the function to return `Option<_>`"
                            .to_string(),
                        NO_CARRIER_RETURN,
                        span,
                    ));
                    return (TypedExprKind::Null, InferType::Dynamic);
                }
            }
        }

        let enum_name = carrier.enum_name().to_string();
        let ok_variant = match &carrier {
            Carrier::Result { .. } => "Ok",
            Carrier::Option { .. } => "Some",
        };

        let v_name = self.next_try_binding('v');
        let ok_arm = MatchArm {
            pattern: variant_pattern(&enum_name, ok_variant, vec![v_name.clone()], span),
            body: Box::new(ident_expr(&v_name, span)),
            span,
        };

        let prop_arm = match &carrier {
            Carrier::Result { .. } => {
                let e_name = self.next_try_binding('e');
                MatchArm {
                    pattern: variant_pattern(&enum_name, "Err", vec![e_name.clone()], span),
                    body: Box::new(return_variant_block(
                        &enum_name,
                        "Err",
                        vec![ident_expr(&e_name, span)],
                        span,
                    )),
                    span,
                }
            }
            Carrier::Option { .. } => MatchArm {
                pattern: variant_pattern(&enum_name, "None", vec![], span),
                body: Box::new(return_variant_block(&enum_name, "None", vec![], span)),
                span,
            },
        };

        let arms = vec![ok_arm, prop_arm];
        self.infer_match_typed(typed_inner, inner.span, &arms, span, false)
    }

    // $ is rejected by the identifier scanner, so these names never collide with user code
    fn next_try_binding(&mut self, tag: char) -> String {
        let n = self.try_counter;
        self.try_counter += 1;
        format!("__{tag}${n}")
    }
}

fn classify_carrier(ty: &InferType) -> Result<Carrier, &'static str> {
    match ty {
        InferType::Enum(name, args) if source_type_name(name) == "Result" && args.len() == 2 => {
            Ok(Carrier::Result {
                enum_name: name.clone(),
                error: args[1].clone(),
            })
        }
        InferType::Enum(name, args) if source_type_name(name) == "Option" && args.len() == 1 => {
            Ok(Carrier::Option {
                enum_name: name.clone(),
            })
        }
        InferType::Var(_) => Err(AMBIGUOUS),
        InferType::Enum(name, _)
            if source_type_name(name) == "Result" || source_type_name(name) == "Option" =>
        {
            Err(AMBIGUOUS)
        }
        _ => Err("[?-stage1] the `?` operator expects a `Result<T, E>` or `Option<T>` value"),
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

fn return_variant_block(enum_name: &str, variant: &str, args: Vec<Expr>, span: Span) -> Expr {
    let construct = Expr::new(
        ExprKind::EnumVariant {
            enum_name: enum_name.to_string(),
            variant: variant.to_string(),
            args,
        },
        span,
    );
    let ret_stmt = Stmt::new(StmtKind::Return(Some(construct)), span);
    Expr::new(
        ExprKind::Block {
            stmts: vec![ret_stmt],
            tail: Box::new(Expr::new(ExprKind::Null, span)),
        },
        span,
    )
}
