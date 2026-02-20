use super::DeadCodeEliminator;
use aelys_sema::{TypedExpr, TypedExprKind};

impl DeadCodeEliminator {
    pub(super) fn eliminate_in_expr(&mut self, expr: &mut TypedExpr) {
        match &mut expr.kind {
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // ternary with constant condition -> just the result
                if let Some(cond_value) = Self::is_const_bool(condition) {
                    let taken = if cond_value { then_branch } else { else_branch };
                    self.eliminate_in_expr(taken);
                    *expr = (**taken).clone();
                    self.stats.branches_eliminated += 1;
                    return;
                }
                self.eliminate_in_expr(condition);
                self.eliminate_in_expr(then_branch);
                self.eliminate_in_expr(else_branch);
            }
            TypedExprKind::Binary { left, right, .. } => {
                self.eliminate_in_expr(left);
                self.eliminate_in_expr(right);
            }
            TypedExprKind::Unary { operand, .. } => self.eliminate_in_expr(operand),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.eliminate_in_expr(left);
                self.eliminate_in_expr(right);
            }
            TypedExprKind::Call { callee, args } => {
                self.eliminate_in_expr(callee);
                for arg in args {
                    self.eliminate_in_expr(arg);
                }
            }
            TypedExprKind::Assign { value, .. } => self.eliminate_in_expr(value),
            TypedExprKind::Grouping(inner) => self.eliminate_in_expr(inner),
            TypedExprKind::Lambda(inner) => self.eliminate_in_expr(inner),
            TypedExprKind::LambdaInner { body, .. } => {
                self.eliminate_in_block(body);
            }
            TypedExprKind::Member { object, .. } => self.eliminate_in_expr(object),
            TypedExprKind::ArrayLiteral { elements, .. }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for elem in elements {
                    self.eliminate_in_expr(elem);
                }
            }
            TypedExprKind::ArraySized { size, .. } => {
                self.eliminate_in_expr(size);
            }
            TypedExprKind::Index { object, index } => {
                self.eliminate_in_expr(object);
                self.eliminate_in_expr(index);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.eliminate_in_expr(object);
                self.eliminate_in_expr(index);
                self.eliminate_in_expr(value);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.eliminate_in_expr(s);
                }
                if let Some(e) = end {
                    self.eliminate_in_expr(e);
                }
            }
            TypedExprKind::Slice { object, range } => {
                self.eliminate_in_expr(object);
                self.eliminate_in_expr(range);
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                        self.eliminate_in_expr(e);
                    }
                }
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.eliminate_in_expr(value);
                }
            }
            TypedExprKind::Cast { expr, .. } => {
                self.eliminate_in_expr(expr);
            }
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null
            | TypedExprKind::Identifier(_) => {}
        }
    }
}
