use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Span, Stmt};

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

        self.env.push_scope();
        let saved_then_literals = self.literal_init_vars.clone();
        let typed_then = self.infer_stmt(then_branch);
        self.env.pop_scope();
        self.literal_init_vars = saved_then_literals;

        let typed_else = else_branch.map(|e| {
            self.env.push_scope();
            let saved_else_literals = self.literal_init_vars.clone();
            let typed = self.infer_stmt(e);
            self.env.pop_scope();
            self.literal_init_vars = saved_else_literals;
            Box::new(typed)
        });

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
            InferType::I64,
            start.span,
            ConstraintReason::ForBounds,
        ));
        self.constraints.push(Constraint::equal(
            typed_end.ty.clone(),
            InferType::I64,
            end.span,
            ConstraintReason::ForBounds,
        ));
        if let Some(ref ts) = typed_step {
            let step_span = step.map(|s| s.span).unwrap_or(body.span);
            self.constraints.push(Constraint::equal(
                ts.ty.clone(),
                InferType::I64,
                step_span,
                ConstraintReason::ForBounds,
            ));
        }

        self.env.push_scope();
        let saved_for_literals = self.literal_init_vars.clone();
        self.env.define_local(iterator.to_string(), InferType::I64);
        let typed_body = self.infer_stmt(body);
        self.env.pop_scope();
        self.literal_init_vars = saved_for_literals;

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
        let typed_iterable = match &iterable.kind {
            ExprKind::Call { callee, args } if args.is_empty() => match &callee.kind {
                ExprKind::Member { object, member } if member == "chars" || member == "bytes" => {
                    let typed_object = self.infer_expr(object);
                    if typed_object.ty.is_str() && member == "chars" {
                        typed_object
                    } else if typed_object.ty.is_str() {
                        let span = typed_object.span;
                        TypedExpr::new(
                            TypedExprKind::Member {
                                object: Box::new(typed_object),
                                member: "bytes".to_string(),
                            },
                            InferType::Slice {
                                elem: Box::new(InferType::U8),
                                mutable: false,
                            },
                            span,
                        )
                    } else {
                        let pretyped = self.stash_pretyped(typed_object);
                        let call = Expr::new(
                            ExprKind::Call {
                                callee: Box::new(Expr::new(
                                    ExprKind::Member {
                                        object: Box::new(pretyped),
                                        member: member.clone(),
                                    },
                                    callee.span,
                                )),
                                args: Vec::new(),
                            },
                            iterable.span,
                        );
                        self.infer_expr(&call)
                    }
                }
                _ => self.infer_expr(iterable),
            },
            _ => self.infer_expr(iterable),
        };

        let elem_type = match &typed_iterable.ty {
            InferType::String | InferType::StrRef => InferType::Char,
            InferType::Vec(inner) => (**inner).clone(),
            InferType::Array(inner, _) => (**inner).clone(),
            InferType::Slice { elem, .. } => (**elem).clone(),
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.defer_read(
                &typed_iterable.ty,
                crate::infer::projection::ProjKind::Element,
                typed_iterable.span,
            ),
            _other => InferType::Dynamic,
        };

        self.env.push_scope();
        let saved_foreach_literals = self.literal_init_vars.clone();
        self.env
            .define_local(iterator.to_string(), elem_type.clone());
        let typed_body = self.infer_stmt(body);
        self.env.pop_scope();
        self.literal_init_vars = saved_foreach_literals;

        TypedStmtKind::ForEach {
            iterator: iterator.to_string(),
            iterable: typed_iterable,
            elem_type,
            body: Box::new(typed_body),
        }
    }
}
