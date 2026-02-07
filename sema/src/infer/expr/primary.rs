use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::Span;

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
                InferType::Dynamic
            });

        (TypedExprKind::Identifier(name.to_string()), ty)
    }
}
