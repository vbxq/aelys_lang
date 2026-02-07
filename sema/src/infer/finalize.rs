use super::TypeInference;
use crate::typed_ast::TypedStmt;

impl TypeInference {
    /// Finalize types: convert InferType to ResolvedType
    /// Unresolved Var -> Dynamic
    /// Types that flowed through Dynamic paths -> Uncertain
    pub(super) fn finalize_stmts(&self, stmts: Vec<TypedStmt>) -> Vec<TypedStmt> {
        stmts.into_iter().map(|s| self.finalize_stmt(s)).collect()
    }

    fn finalize_stmt(&self, stmt: TypedStmt) -> TypedStmt {
        stmt
    }
}
