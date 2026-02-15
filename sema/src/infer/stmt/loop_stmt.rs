use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::TypedStmtKind;
use crate::types::InferType;
use aelys_syntax::{Expr, Span, Stmt};

impl TypeInference {
    pub(super) fn infer_if_stmt(
        &mut self,
        condition: &Expr,
        then_branch: &Stmt,
        else_branch: Option<&Stmt>,
    ) -> TypedStmtKind {
        let typed_cond = self.infer_expr(condition);

        self.constraints.push(Constraint::equal(
            typed_cond.ty.clone(),
            InferType::Bool,
            condition.span,
            ConstraintReason::IfCondition,
        ));

        let typed_then = self.infer_stmt(then_branch);
        let typed_else = else_branch.map(|e| Box::new(self.infer_stmt(e)));

        TypedStmtKind::If {
            condition: typed_cond,
            then_branch: Box::new(typed_then),
            else_branch: typed_else,
        }
    }

    pub(super) fn infer_while_stmt(&mut self, condition: &Expr, body: &Stmt) -> TypedStmtKind {
        let typed_cond = self.infer_expr(condition);

        self.constraints.push(Constraint::equal(
            typed_cond.ty.clone(),
            InferType::Bool,
            condition.span,
            ConstraintReason::WhileCondition,
        ));

        let typed_body = self.infer_stmt(body);

        TypedStmtKind::While {
            condition: typed_cond,
            body: Box::new(typed_body),
        }
    }

    pub(super) fn infer_for_stmt(
        &mut self,
        iterator: &str,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        step: Option<&Expr>,
        body: &Stmt,
    ) -> TypedStmtKind {
        let typed_start = self.infer_expr(start);
        let typed_end = self.infer_expr(end);
        let typed_step = step.map(|s| self.infer_expr(s));

        self.constraints.push(Constraint::equal(
            typed_start.ty.clone(),
            InferType::Int,
            start.span,
            ConstraintReason::ForBounds,
        ));
        self.constraints.push(Constraint::equal(
            typed_end.ty.clone(),
            InferType::Int,
            end.span,
            ConstraintReason::ForBounds,
        ));
        if let Some(ref ts) = typed_step {
            let step_span = step.map(|s| s.span).unwrap_or(body.span);
            self.constraints.push(Constraint::equal(
                ts.ty.clone(),
                InferType::Int,
                step_span,
                ConstraintReason::ForBounds,
            ));
        }

        self.env.push_scope();
        self.env.define_local(iterator.to_string(), InferType::Int);
        let typed_body = self.infer_stmt(body);
        self.env.pop_scope();

        TypedStmtKind::For {
            iterator: iterator.to_string(),
            start: typed_start,
            end: typed_end,
            inclusive,
            step: Box::new(typed_step),
            body: Box::new(typed_body),
        }
    }

    pub(super) fn infer_for_each_stmt(
        &mut self,
        iterator: &str,
        iterable: &Expr,
        body: &Stmt,
        _span: Span,
    ) -> TypedStmtKind {
        let typed_iterable = self.infer_expr(iterable);

        let elem_type = match &typed_iterable.ty {
            InferType::String => InferType::String,
            InferType::Vec(inner) => (**inner).clone(),
            InferType::Array(inner) => (**inner).clone(),
            InferType::Dynamic => InferType::Dynamic,
            _ => InferType::Dynamic,
        };

        self.env.push_scope();
        self.env
            .define_local(iterator.to_string(), elem_type.clone());
        let typed_body = self.infer_stmt(body);
        self.env.pop_scope();

        TypedStmtKind::ForEach {
            iterator: iterator.to_string(),
            iterable: typed_iterable,
            elem_type,
            body: Box::new(typed_body),
        }
    }
}
