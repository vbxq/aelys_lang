use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExprKind, TypedStmtKind};
use aelys_syntax::{Expr, Span, TypeAnnotation, UnaryOp};

impl TypeInference {
    pub(super) fn infer_let_stmt(
        &mut self,
        span: Span,
        name: &str,
        mutable: bool,
        type_annotation: &Option<TypeAnnotation>,
        initializer: &Expr,
        is_pub: bool,
    ) -> TypedStmtKind {
        let mut typed_init = self.infer_expr(initializer);

        let declared_type = type_annotation
            .as_ref()
            .map(|ann| self.type_from_annotation(ann));

        let var_type = if let Some(decl) = &declared_type {
            // try to narrow numeric literal to match declared type.
            // always push a constraint afterwards so the solver validates the narrowing decision.
            // narrowing alone must never be the sole source of truth for a type
            self.try_narrow_literal(&mut typed_init, decl);

            // Implicit numeric widening for let initializer
            if typed_init.ty != *decl
                && typed_init.ty.can_implicit_widen_to(decl)
            {
                let vspan = typed_init.span;
                let original = std::mem::replace(
                    &mut typed_init,
                    crate::typed_ast::TypedExpr {
                        kind: crate::typed_ast::TypedExprKind::Null,
                        ty: crate::types::InferType::Null,
                        span: vspan,
                    },
                );
                typed_init = crate::typed_ast::TypedExpr {
                    kind: crate::typed_ast::TypedExprKind::Cast {
                        expr: Box::new(original),
                        target: decl.clone(),
                    },
                    ty: decl.clone(),
                    span: vspan,
                };
            }

            self.constraints.push(Constraint::equal(
                typed_init.ty.clone(),
                decl.clone(),
                span,
                ConstraintReason::TypeAnnotation {
                    var_name: name.to_string(),
                },
            ));
            decl.clone()
        } else {
            // track immutable variables initialized with numeric literals so that try_narrow_literal can narrow through variable references at return/call/assign sites.
            //
            // mutable variables are not tracked because they can be reassigned to a different value that might not fit in the target type
            if !mutable {
                Self::track_literal_init(&mut self.literal_init_vars, name, &typed_init);
            }
            typed_init.ty.clone()
        };

        self.env
            .define_local_with_span(name.to_string(), var_type.clone(), span);
        if mutable {
            self.env.mark_mutable(name.to_string());
        }

        TypedStmtKind::Let {
            name: name.to_string(),
            mutable,
            initializer: typed_init,
            var_type,
            is_pub,
        }
    }

    /// Record that a variable was initialized with a numeric literal value.
    /// That info is used by `try_narrow_literal` to narrow through variable references
    ///
    /// we also tracks variable to variable copies, like if `x` is tracked and `let y = x` is encountered, `y` inherits the same literal value
    fn track_literal_init(
        literal_init_vars: &mut std::collections::HashMap<String, LiteralInit>,
        name: &str,
        init: &crate::typed_ast::TypedExpr,
    ) {
        match &init.kind {
            TypedExprKind::Int(v) => {
                literal_init_vars.insert(name.to_string(), LiteralInit::Int(*v));
            }
            TypedExprKind::Float(v) => {
                literal_init_vars.insert(name.to_string(), LiteralInit::Float(*v));
            }
            TypedExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => match &operand.kind {
                TypedExprKind::Int(v) => {
                    if let Some(neg) = v.checked_neg() {
                        literal_init_vars.insert(name.to_string(), LiteralInit::Int(neg));
                    }
                }
                TypedExprKind::Float(v) => {
                    literal_init_vars.insert(name.to_string(), LiteralInit::Float(-v));
                }
                _ => {}
            },
            // propagate literal tracking through variable copies
            // `let y = x` where x was tracked -> y gets the same literal value
            TypedExprKind::Identifier(source_name) => {
                if let Some(lit) = literal_init_vars.get(source_name).cloned() {
                    literal_init_vars.insert(name.to_string(), lit);
                }
            }
            // track if-else expressions where both branches are known integer  or float literals. use the branch with the larger absolute value so narrowing checks the worst case.
            TypedExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let Some(lit) = Self::extract_if_else_literal(then_branch, else_branch) {
                    literal_init_vars.insert(name.to_string(), lit);
                }
            }
            _ => {}
        }
    }

    /// Extract a LiteralInit from an if-else where both branches are known
    /// integer or float literals.
    ///
    /// Returns the branch with the larger absolute value so that narrowing overflow checks are conservative.
    fn extract_if_else_literal(
        then_branch: &crate::typed_ast::TypedExpr,
        else_branch: &crate::typed_ast::TypedExpr,
    ) -> Option<LiteralInit> {
        match (&then_branch.kind, &else_branch.kind) {
            (TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
                let worst = if a.abs() >= b.abs() { *a } else { *b };
                Some(LiteralInit::Int(worst))
            }
            (TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
                let worst = if a.abs() >= b.abs() { *a } else { *b };
                Some(LiteralInit::Float(worst))
            }
            _ => None,
        }
    }
}

/// tracks the literal value a variable was initialized with.
#[derive(Debug, Clone)]
pub enum LiteralInit {
    Int(i64),
    Float(f64),
}
