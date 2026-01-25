use super::TypeInference;
use crate::typed_ast::TypedStmt;
use crate::unify::Substitution;

mod expr;
mod func;
mod stmt;

impl TypeInference {
    /// Apply substitution to typed statements
    pub(super) fn apply_substitution_stmts(
        &self,
        stmts: &[TypedStmt],
        subst: &Substitution,
    ) -> Vec<TypedStmt> {
        stmts
            .iter()
            .map(|s| self.apply_substitution_stmt(s, subst))
            .collect()
    }
}
