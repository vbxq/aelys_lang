use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::Stmt;

impl TypeInference {
    /// Infer statement with implicit return handling
    pub(crate) fn infer_stmt_with_implicit_return(
        &mut self,
        stmt: &Stmt,
        return_type: &InferType,
    ) -> TypedStmt {
        match &stmt.kind {
            aelys_syntax::StmtKind::Expression(expr) => {
                let typed_expr = self.infer_expr(expr);

                self.constraints.push(Constraint::equal(
                    typed_expr.ty.clone(),
                    return_type.clone(),
                    expr.span,
                    ConstraintReason::Return {
                        func_name: self
                            .env
                            .current_function()
                            .cloned()
                            .unwrap_or_else(|| "<anonymous>".to_string()),
                    },
                ));

                TypedStmt {
                    kind: TypedStmtKind::Expression(typed_expr),
                    span: stmt.span,
                }
            }

            aelys_syntax::StmtKind::If {
                condition,
                then_branch,
                else_branch: Some(else_branch),
            } => {
                let typed_cond = self.infer_expr(condition);

                self.constraints.push(Constraint::equal(
                    typed_cond.ty.clone(),
                    InferType::Bool,
                    condition.span,
                    ConstraintReason::IfCondition,
                ));

                let typed_then = self.infer_stmt_with_implicit_return(then_branch, return_type);
                let typed_else = self.infer_stmt_with_implicit_return(else_branch, return_type);

                TypedStmt {
                    kind: TypedStmtKind::If {
                        condition: typed_cond,
                        then_branch: Box::new(typed_then),
                        else_branch: Some(Box::new(typed_else)),
                    },
                    span: stmt.span,
                }
            }

            aelys_syntax::StmtKind::Block(stmts) if !stmts.is_empty() => {
                self.env.push_scope();

                let mut typed_stmts: Vec<TypedStmt> = stmts[..stmts.len() - 1]
                    .iter()
                    .map(|s| self.infer_stmt(s))
                    .collect();

                let typed_last =
                    self.infer_stmt_with_implicit_return(&stmts[stmts.len() - 1], return_type);
                typed_stmts.push(typed_last);

                self.env.pop_scope();

                TypedStmt {
                    kind: TypedStmtKind::Block(typed_stmts),
                    span: stmt.span,
                }
            }

            _ => self.infer_stmt(stmt),
        }
    }
}
