use super::TypeInference;
use crate::constraint::TypeError;
use crate::modules::source_type_name;
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::{CatchHandler, Expr, ExprKind, MatchArm, Pattern, Span};

impl TypeInference {
    pub(super) fn infer_catch_expr(
        &mut self,
        scrutinee: &Expr,
        handler: &CatchHandler,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_scrutinee = self.infer_expr(scrutinee);

        if matches!(typed_scrutinee.ty, InferType::Dynamic) {
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        let (enum_name, t) = match &typed_scrutinee.ty {
            InferType::Enum(name, targs)
                if source_type_name(name) == "Result" && targs.len() == 2 =>
            {
                (name.clone(), targs[0].clone())
            }
            other => {
                self.errors.push(TypeError::error_handling(
                    format!("[eh-stage3] `catch` requires a `Result<T, E>` value, found `{other}`"),
                    "not a `Result` value",
                    scrutinee.span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
        };

        let obj_span = typed_scrutinee.span;
        let v_name = self.next_catch_binding('v');
        let ok_arm = MatchArm {
            pattern: variant_pattern(&enum_name, "Ok", vec![v_name.clone()], span),
            body: Box::new(ident_expr(&v_name, span)),
            span,
        };

        let err_arm = match handler {
            CatchHandler::Binding { name, body } => MatchArm {
                pattern: variant_pattern(&enum_name, "Err", vec![name.clone()], span),
                body: body.clone(),
                span,
            },
            CatchHandler::Arms(arms) => {
                self.catch_match_pending = true;
                let e_name = self.next_catch_binding('e');
                let inner = Expr::new(
                    ExprKind::Match {
                        scrutinee: Box::new(ident_expr(&e_name, span)),
                        arms: arms.clone(),
                    },
                    span,
                );
                MatchArm {
                    pattern: variant_pattern(&enum_name, "Err", vec![e_name], span),
                    body: Box::new(inner),
                    span,
                }
            }
        };

        let arms = vec![ok_arm, err_arm];
        let (kind, _) = self.infer_match_typed(typed_scrutinee, obj_span, &arms, span, false);
        // pin t so must-use never fires: catch removes e, so the value is no longer a result
        (kind, t)
    }

    // $ is rejected by the scanner, so these hygienic names never collide with user code
    fn next_catch_binding(&mut self, tag: char) -> String {
        let n = self.try_counter;
        self.try_counter += 1;
        format!("__{tag}${n}")
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
