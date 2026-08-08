// typedexprkind is rejected by default rather than accepted by omission.
use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmt, TypedStmtKind};
use crate::types::InferType;

impl TypeInference {
    pub(super) fn check_vec_producing_forms(&mut self, stmts: &[TypedStmt]) {
        for stmt in stmts {
            self.vec_form_stmt(stmt);
        }
    }

    fn vec_form_stmt(&mut self, stmt: &TypedStmt) {
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => self.vec_form_expr(expr),
            TypedStmtKind::Let { initializer, .. } => {
                self.check_vec_producing(initializer, "initializer");
                self.vec_form_expr(initializer);
            }
            TypedStmtKind::Block(inner) => {
                for s in inner {
                    self.vec_form_stmt(s);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.vec_form_expr(condition);
                self.vec_form_stmt(then_branch);
                if let Some(else_branch) = else_branch {
                    self.vec_form_stmt(else_branch);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.vec_form_expr(condition);
                self.vec_form_stmt(body);
            }
            TypedStmtKind::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.vec_form_expr(start);
                self.vec_form_expr(end);
                if let Some(step) = step.as_ref().as_ref() {
                    self.vec_form_expr(step);
                }
                self.vec_form_stmt(body);
            }
            TypedStmtKind::ForEach { iterable, body, .. } => {
                self.vec_form_expr(iterable);
                self.vec_form_stmt(body);
            }
            TypedStmtKind::Return(Some(expr)) => {
                self.check_vec_producing(expr, "returned expression");
                self.vec_form_expr(expr);
            }
            TypedStmtKind::Function(func) => {
                for s in &func.body {
                    self.vec_form_stmt(s);
                }
            }
            TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }

    fn vec_form_expr(&mut self, expr: &TypedExpr) {
        match &expr.kind {
            TypedExprKind::Assign { value, .. } => {
                self.check_vec_producing(value, "assigned value");
                self.vec_form_expr(value);
            }
            TypedExprKind::DerefAssign { target, value } => {
                self.check_vec_producing(value, "assigned value");
                self.vec_form_expr(target);
                self.vec_form_expr(value);
            }
            TypedExprKind::Binary { left, right, .. }
            | TypedExprKind::And { left, right }
            | TypedExprKind::Or { left, right } => {
                self.vec_form_expr(left);
                self.vec_form_expr(right);
            }
            TypedExprKind::Unary { operand, .. }
            | TypedExprKind::Reference { operand, .. } => self.vec_form_expr(operand),
            TypedExprKind::Grouping(inner)
            | TypedExprKind::Deref(inner)
            | TypedExprKind::Lambda(inner) => self.vec_form_expr(inner),
            TypedExprKind::Call { callee, args } => {
                self.vec_form_expr(callee);
                for arg in args {
                    self.vec_form_expr(arg);
                }
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.vec_form_expr(condition);
                self.vec_form_expr(then_branch);
                self.vec_form_expr(else_branch);
            }
            TypedExprKind::LambdaInner { body, .. } => {
                for s in body {
                    self.vec_form_stmt(s);
                }
            }
            TypedExprKind::Member { object, .. } => self.vec_form_expr(object),
            TypedExprKind::ArrayLiteral { elements }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for e in elements {
                    self.vec_form_expr(e);
                }
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                self.vec_form_expr(size);
                if let Some(fill) = fill_value {
                    self.vec_form_expr(fill);
                }
            }
            TypedExprKind::Index { object, index } => {
                self.vec_form_expr(object);
                self.vec_form_expr(index);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.vec_form_expr(object);
                self.vec_form_expr(index);
                self.vec_form_expr(value);
            }
            TypedExprKind::FieldAssign { object, value, .. } => {
                self.vec_form_expr(object);
                self.vec_form_expr(value);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.vec_form_expr(start);
                }
                if let Some(end) = end {
                    self.vec_form_expr(end);
                }
            }
            TypedExprKind::Slice { object, range } => {
                self.vec_form_expr(object);
                self.vec_form_expr(range);
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.vec_form_expr(value);
                }
            }
            TypedExprKind::Cast { expr, .. } => self.vec_form_expr(expr),
            TypedExprKind::EnumVariant { args, .. } => {
                for arg in args {
                    self.vec_form_expr(arg);
                }
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.vec_form_expr(scrutinee);
                for arm in arms {
                    self.vec_form_expr(&arm.body);
                }
            }
            TypedExprKind::ResultAssert { scrutinee, .. } => self.vec_form_expr(scrutinee),
            TypedExprKind::Block { stmts, tail } => {
                for s in stmts {
                    self.vec_form_stmt(s);
                }
                self.vec_form_expr(tail);
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let TypedFmtStringPart::Expr(inner) = part {
                        self.vec_form_expr(inner);
                    }
                }
            }
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null
            | TypedExprKind::Identifier(_) => {}
        }
    }

    fn check_vec_producing(&mut self, expr: &TypedExpr, position: &str) {
        if !matches!(expr.ty, InferType::Vec(_)) {
            return;
        }
        let accepted = match &expr.kind {
            TypedExprKind::EnumVariant {
                enum_name, variant, ..
            } => enum_name == "Vec" && variant == "new",
            TypedExprKind::VecLiteral { .. } => true,
            TypedExprKind::Identifier(_) => true,
            TypedExprKind::Call { .. } => true,
            _ => false,
        };
        if accepted {
            return;
        }

        let mut error = TypeError::vec_out_of_surface(
            format!(
                "the {position} has type `{}` but is not one of the four supported \
                 Vec-producing forms (`Vec::new()`, a `vec[...]` literal, a bare identifier, \
                 or a call); an indirect form is not supported yet (the buffer would be aliased \
                 by a temporary that owns no share)",
                expr.ty
            ),
            expr.span,
        );
        if matches!(expr.kind, TypedExprKind::Grouping(_)) {
            error.help = Some("remove the parentheses".to_string());
        }
        self.errors.push(error);
    }
}

