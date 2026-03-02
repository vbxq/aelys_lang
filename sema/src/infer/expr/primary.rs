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

                // register the variable with Dynamic type to prevent repeated "undefined variable" errors for each subsequent use
                let recovery_ty = InferType::Dynamic;
                self.env.define_local(name.to_string(), recovery_ty.clone());

                recovery_ty
            });

        (TypedExprKind::Identifier(name.to_string()), ty)
    }
}
