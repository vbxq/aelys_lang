use super::super::Compiler;
use aelys_common::Result;

impl Compiler {
    pub fn compile_typed_expr(&mut self, expr: &aelys_sema::TypedExpr, dest: u8) -> Result<()> {
        use aelys_sema::TypedExprKind;

        match &expr.kind {
            TypedExprKind::Int(n) => self.compile_literal_int(*n, dest, expr.span),
            TypedExprKind::Float(f) => self.compile_literal_float(*f, dest, expr.span),
            TypedExprKind::String(s) => self.compile_literal_string(s, dest, expr.span),
            TypedExprKind::Bool(b) => self.compile_literal_bool(*b, dest, expr.span),
            TypedExprKind::Null => self.compile_literal_null(dest, expr.span),
            TypedExprKind::Identifier(name) => self.compile_identifier(name, dest, expr.span),
            TypedExprKind::Binary { left, op, right } => {
                self.compile_typed_binary(left, *op, right, dest, expr.span)
            }
            TypedExprKind::Unary { op, operand } => {
                self.compile_typed_unary(*op, operand, dest, expr.span)
            }
            TypedExprKind::And { left, right } => {
                self.compile_typed_and(left, right, dest, expr.span)
            }
            TypedExprKind::Or { left, right } => {
                self.compile_typed_or(left, right, dest, expr.span)
            }
            TypedExprKind::Call { callee, args } => {
                self.compile_typed_call(callee, args, dest, expr.span)
            }
            TypedExprKind::Assign { name, value } => {
                self.compile_typed_assign(name, value, dest, expr.span)
            }
            TypedExprKind::Grouping(inner) => self.compile_typed_expr(inner, dest),
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_typed_if_expr(condition, then_branch, else_branch, dest),
            TypedExprKind::Lambda(inner) => self.compile_typed_expr(inner, dest),
            TypedExprKind::LambdaInner {
                params,
                return_type: _,
                body,
                captures,
            } => self.compile_typed_lambda_with_stmts(params, body, captures, dest, expr.span),
            TypedExprKind::Member { object, member } => {
                self.compile_typed_member_access(object, member, dest, expr.span)
            }
        }
    }

    pub(super) fn typed_expr_may_have_side_effects(expr: &aelys_sema::TypedExpr) -> bool {
        use aelys_sema::TypedExprKind;

        match &expr.kind {
            TypedExprKind::Call { .. } => true,
            TypedExprKind::Assign { .. } => true,
            TypedExprKind::Binary { left, right, .. } => {
                Self::typed_expr_may_have_side_effects(left)
                    || Self::typed_expr_may_have_side_effects(right)
            }
            TypedExprKind::Unary { operand, .. } => Self::typed_expr_may_have_side_effects(operand),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                Self::typed_expr_may_have_side_effects(left)
                    || Self::typed_expr_may_have_side_effects(right)
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::typed_expr_may_have_side_effects(condition)
                    || Self::typed_expr_may_have_side_effects(then_branch)
                    || Self::typed_expr_may_have_side_effects(else_branch)
            }
            TypedExprKind::Grouping(inner) | TypedExprKind::Lambda(inner) => {
                Self::typed_expr_may_have_side_effects(inner)
            }
            TypedExprKind::Member { object, .. } => Self::typed_expr_may_have_side_effects(object),
            TypedExprKind::LambdaInner { .. } => false,
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null
            | TypedExprKind::Identifier(_) => false,
        }
    }
}
