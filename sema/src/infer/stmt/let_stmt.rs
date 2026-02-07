use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::TypedStmtKind;
use aelys_syntax::{Expr, Span, TypeAnnotation};

impl TypeInference {
    pub(super) fn infer_let_stmt(
        &mut self,
        span: Span,
        name: &str,
        mutable: bool,
        type_annotation: &Option<TypeAnnotation>,
        initializer: &Expr,
        is_pub: bool,
    ) -> TypedStmtKind {
        let typed_init = self.infer_expr(initializer);

        let declared_type = type_annotation
            .as_ref()
            .map(|ann| self.type_from_annotation(ann));

        let var_type = if let Some(decl) = &declared_type {
            self.constraints.push(Constraint::equal(
                typed_init.ty.clone(),
                decl.clone(),
                span,
                ConstraintReason::TypeAnnotation {
                    var_name: name.to_string(),
                },
            ));
            decl.clone()
        } else {
            typed_init.ty.clone()
        };

        self.env.define_local(name.to_string(), var_type.clone());

        TypedStmtKind::Let {
            name: name.to_string(),
            mutable,
            initializer: typed_init,
            var_type,
            is_pub,
        }
    }
}
