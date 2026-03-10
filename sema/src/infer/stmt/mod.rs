//! Statement inference.

mod block;
mod implicit;
pub(crate) mod let_stmt;
mod loop_stmt;
mod needs;
mod return_stmt;

use super::TypeInference;
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use aelys_syntax::{Stmt, StmtKind};

impl TypeInference {
    /// Infer types for a list of statements
    pub(super) fn infer_stmts(&mut self, stmts: &[Stmt]) -> Vec<TypedStmt> {
        stmts.iter().map(|s| self.infer_stmt(s)).collect()
    }

    /// Infer type for a single statement
    pub(super) fn infer_stmt(&mut self, stmt: &Stmt) -> TypedStmt {
        let kind = match &stmt.kind {
            StmtKind::Expression(expr) => {
                let typed_expr = self.infer_expr(expr);
                TypedStmtKind::Expression(typed_expr)
            }
            StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub,
            } => self.infer_let_stmt(
                stmt.span,
                name,
                *mutable,
                type_annotation,
                initializer,
                *is_pub,
            ),
            StmtKind::Block(stmts) => self.infer_block_stmt(stmts),
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.infer_if_stmt(condition, then_branch, else_branch.as_deref()),
            StmtKind::While { condition, body } => self.infer_while_stmt(condition, body),
            StmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => self.infer_for_stmt(
                iterator,
                start,
                end,
                *inclusive,
                step.as_ref().as_ref(),
                body,
            ),
            StmtKind::ForEach {
                iterator,
                iterable,
                body,
            } => self.infer_for_each_stmt(iterator, iterable, body, stmt.span),
            StmtKind::Return(expr) => self.infer_return_stmt(stmt.span, expr.as_ref()),
            StmtKind::Break => TypedStmtKind::Break,
            StmtKind::Continue => TypedStmtKind::Continue,
            StmtKind::Function(func) => {
                let typed_func = self.infer_function(func);
                TypedStmtKind::Function(typed_func)
            }
            StmtKind::Needs(needs) => {
                self.handle_needs_stmt(needs);
                TypedStmtKind::Needs(needs.clone())
            }
            StmtKind::StructDecl {
                name,
                type_params,
                fields,
                ..
            } => {
                // temporarily set type_params_in_scope so that generic struct field types like `T` don't trigger unknown-type errors
                // TODO: !
                let saved = std::mem::replace(&mut self.type_params_in_scope, type_params.clone());
                let typed_fields = fields
                    .iter()
                    .map(|f| {
                        let ty = self.type_from_annotation(&f.type_annotation);
                        (f.name.clone(), ty)
                    })
                    .collect();
                self.type_params_in_scope = saved;
                TypedStmtKind::StructDecl {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    fields: typed_fields,
                }
            }
            StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                ..
            } => {
                // Look up the data types from the type table (populated by collect_enums)
                let enum_def = self.type_table.get_enum(name).cloned();
                TypedStmtKind::EnumDecl {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: variants
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let data = enum_def
                                .as_ref()
                                .and_then(|def| def.variants.iter().find(|ev| ev.name == v.name))
                                .map(|ev| ev.data.clone())
                                .unwrap_or_default();
                            (v.name.clone(), i as u32, data)
                        })
                        .collect(),
                }
            }
        };

        TypedStmt {
            kind,
            span: stmt.span,
        }
    }
}
