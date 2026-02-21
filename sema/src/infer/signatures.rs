use super::TypeInference;
use crate::types::InferType;
use aelys_syntax::{Function, Stmt, StmtKind};
use std::rc::Rc;

impl TypeInference {
    /// Collect function signatures before inference (pre-pass)
    pub(super) fn collect_signatures(&mut self, stmts: &[Stmt], prefix: &str) {
        for stmt in stmts {
            match &stmt.kind {
                StmtKind::Function(func) => {
                    self.collect_function_signature(func, prefix);
                }
                StmtKind::Block(inner_stmts) => {
                    self.collect_signatures(inner_stmts, prefix);
                }
                StmtKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    self.collect_signatures_from_stmt(then_branch, prefix);
                    if let Some(else_branch) = else_branch {
                        self.collect_signatures_from_stmt(else_branch, prefix);
                    }
                }
                StmtKind::While { body, .. } => {
                    self.collect_signatures_from_stmt(body, prefix);
                }
                StmtKind::For { body, .. } => {
                    self.collect_signatures_from_stmt(body, prefix);
                }
                StmtKind::ForEach { body, .. } => {
                    self.collect_signatures_from_stmt(body, prefix);
                }
                _ => {}
            }
        }
    }

    /// Collect signatures from a single statement
    fn collect_signatures_from_stmt(&mut self, stmt: &Stmt, prefix: &str) {
        match &stmt.kind {
            StmtKind::Function(func) => {
                self.collect_function_signature(func, prefix);
            }
            StmtKind::Block(stmts) => {
                self.collect_signatures(stmts, prefix);
            }
            _ => {}
        }
    }

    /// Collect a single function's signature
    fn collect_function_signature(&mut self, func: &Function, prefix: &str) {
        let full_name = if prefix.is_empty() {
            func.name.clone()
        } else {
            format!("{}::{}", prefix, func.name)
        };

        let saved_type_params = std::mem::replace(
            &mut self.type_params_in_scope,
            func.type_params.clone(),
        );

        let mut param_types = Vec::with_capacity(func.params.len());
        for p in &func.params {
            let ty = match &p.type_annotation {
                Some(ann) => self.type_from_annotation(ann),
                None => self.type_gen.fresh(),
            };
            param_types.push(ty);
        }

        let ret_type = match &func.return_type {
            Some(ann) => self.type_from_annotation(ann),
            None => self.type_gen.fresh(),
        };

        self.type_params_in_scope = saved_type_params;

        let fn_type = Rc::new(InferType::Function {
            params: param_types,
            ret: Box::new(ret_type),
        });

        self.env.define_function(full_name, fn_type);
    }
}
