use super::super::TypeInference;
use crate::typed_ast::{TypedFunction, TypedParam};
use crate::unify::Substitution;

impl TypeInference {
    /// Apply substitution to function
    pub(super) fn apply_substitution_func(
        &self,
        func: &TypedFunction,
        subst: &Substitution,
    ) -> TypedFunction {
        TypedFunction {
            name: func.name.clone(),
            params: func
                .params
                .iter()
                .map(|p| TypedParam {
                    name: p.name.clone(),
                    ty: subst.apply(&p.ty),
                    span: p.span,
                })
                .collect(),
            return_type: subst.apply(&func.return_type),
            body: self.apply_substitution_stmts(&func.body, subst),
            decorators: func.decorators.clone(),
            is_pub: func.is_pub,
            span: func.span,
            captures: func
                .captures
                .iter()
                .map(|(name, ty)| (name.clone(), subst.apply(ty)))
                .collect(),
        }
    }
}
