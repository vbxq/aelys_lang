use super::TypeInference;
use crate::typed_ast::TypedStmtKind;
use aelys_syntax::Stmt;

impl TypeInference {
    pub(super) fn infer_block_stmt(&mut self, stmts: &[Stmt]) -> TypedStmtKind {
        self.env.push_scope();
        let saved_literal_inits = self.literal_init_vars.clone();
        let typed_stmts = self.infer_stmts(stmts);
        self.env.pop_scope();
        self.literal_init_vars = saved_literal_inits;
        TypedStmtKind::Block(typed_stmts)
    }
}
