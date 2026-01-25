use super::TypeInference;
use crate::typed_ast::TypedStmtKind;
use aelys_syntax::Stmt;

impl TypeInference {
    pub(super) fn infer_block_stmt(&mut self, stmts: &[Stmt]) -> TypedStmtKind {
        self.env.push_scope();
        let typed_stmts = self.infer_stmts(stmts);
        self.env.pop_scope();
        TypedStmtKind::Block(typed_stmts)
    }
}
