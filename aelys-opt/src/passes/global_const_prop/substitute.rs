use super::GlobalConstantPropagator;
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};

impl GlobalConstantPropagator {
    pub(super) fn substitute_constants(&mut self, expr: &mut TypedExpr) {
        match &mut expr.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(c) = self.constants.get(name) {
                    *expr = TypedExpr::new(c.kind.clone(), c.ty.clone(), expr.span);
                    self.stats.globals_propagated += 1;
                }
            }
            TypedExprKind::Binary { left, right, .. } => {
                self.substitute_constants(left);
                self.substitute_constants(right);
            }
            TypedExprKind::Unary { operand, .. } => self.substitute_constants(operand),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.substitute_constants(left);
                self.substitute_constants(right);
            }
            TypedExprKind::Call { callee, args } => {
                self.substitute_constants(callee);
                for arg in args {
                    self.substitute_constants(arg);
                }
            }
            TypedExprKind::Assign { value, .. } => self.substitute_constants(value),
            TypedExprKind::Grouping(inner) => self.substitute_constants(inner),
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.substitute_constants(condition);
                self.substitute_constants(then_branch);
                self.substitute_constants(else_branch);
            }
            TypedExprKind::Lambda(inner) => self.substitute_constants(inner),
            TypedExprKind::LambdaInner { body, .. } => {
                for stmt in body {
                    self.substitute_in_stmt(stmt);
                }
            }
            TypedExprKind::Member { object, .. } => self.substitute_constants(object),
            TypedExprKind::ArrayLiteral { elements, .. }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for elem in elements {
                    self.substitute_constants(elem);
                }
            }
            TypedExprKind::ArraySized { size, .. } => {
                self.substitute_constants(size);
            }
            TypedExprKind::Index { object, index } => {
                self.substitute_constants(object);
                self.substitute_constants(index);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.substitute_constants(object);
                self.substitute_constants(index);
                self.substitute_constants(value);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.substitute_constants(s);
                }
                if let Some(e) = end {
                    self.substitute_constants(e);
                }
            }
            TypedExprKind::Slice { object, range } => {
                self.substitute_constants(object);
                self.substitute_constants(range);
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                        self.substitute_constants(e);
                    }
                }
            }
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null => {}
        }
    }

    pub(super) fn substitute_in_stmt(&mut self, stmt: &mut TypedStmt) {
        match &mut stmt.kind {
            TypedStmtKind::Expression(expr) => self.substitute_constants(expr),
            TypedStmtKind::Let { initializer, .. } => self.substitute_constants(initializer),
            TypedStmtKind::Block(stmts) => {
                for s in stmts {
                    self.substitute_in_stmt(s);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.substitute_constants(condition);
                self.substitute_in_stmt(then_branch);
                if let Some(else_b) = else_branch {
                    self.substitute_in_stmt(else_b);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.substitute_constants(condition);
                self.substitute_in_stmt(body);
            }
            TypedStmtKind::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.substitute_constants(start);
                self.substitute_constants(end);
                if let Some(s) = &mut **step {
                    self.substitute_constants(s);
                }
                self.substitute_in_stmt(body);
            }
            TypedStmtKind::Return(Some(expr)) => self.substitute_constants(expr),
            TypedStmtKind::Function(func) => self.substitute_in_function(func),
            TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_) => {}
        }
    }

    fn substitute_in_function(&mut self, func: &mut TypedFunction) {
        for stmt in &mut func.body {
            self.substitute_in_stmt(stmt);
        }
    }
}
