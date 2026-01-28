use super::scope::ScopeStack;
use crate::passes::{ConstantFolder, OptimizationPass, OptimizationStats};
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedProgram, TypedStmt, TypedStmtKind};

pub struct LocalConstantPropagator {
    scopes: ScopeStack,
    folder: ConstantFolder,
    stats: OptimizationStats,
}

impl LocalConstantPropagator {
    pub fn new() -> Self {
        Self {
            scopes: ScopeStack::new(),
            folder: ConstantFolder::new(),
            stats: OptimizationStats::new(),
        }
    }

    fn is_simple_constant(expr: &TypedExpr) -> bool {
        matches!(
            &expr.kind,
            TypedExprKind::Int(_)
                | TypedExprKind::Float(_)
                | TypedExprKind::Bool(_)
                | TypedExprKind::String(_)
                | TypedExprKind::Null
        )
    }

    fn propagate_stmt(&mut self, stmt: &mut TypedStmt) {
        match &mut stmt.kind {
            TypedStmtKind::Let { name, mutable, initializer, .. } => {
                self.propagate_expr(initializer);
                self.folder.optimize_expr(initializer);

                if !*mutable && Self::is_simple_constant(initializer) {
                    self.scopes.insert(name.clone(), initializer.clone());
                }
            }

            TypedStmtKind::Expression(expr) => {
                self.propagate_expr(expr);
            }

            TypedStmtKind::Block(stmts) => {
                self.scopes.push();
                for s in stmts.iter_mut() {
                    self.propagate_stmt(s);
                }
                self.scopes.pop();
            }

            TypedStmtKind::If { condition, then_branch, else_branch } => {
                self.propagate_expr(condition);
                self.scopes.push();
                self.propagate_stmt(then_branch);
                self.scopes.pop();
                if let Some(else_b) = else_branch {
                    self.scopes.push();
                    self.propagate_stmt(else_b);
                    self.scopes.pop();
                }
            }

            TypedStmtKind::While { condition, body } => {
                self.propagate_expr(condition);
                self.scopes.push();
                self.propagate_stmt(body);
                self.scopes.pop();
            }

            TypedStmtKind::For { start, end, step, body, .. } => {
                self.propagate_expr(start);
                self.propagate_expr(end);
                if let Some(s) = step {
                    self.propagate_expr(s);
                }
                self.scopes.push();
                self.propagate_stmt(body);
                self.scopes.pop();
            }

            TypedStmtKind::Return(Some(expr)) => {
                self.propagate_expr(expr);
            }

            TypedStmtKind::Function(func) => {
                self.propagate_function(func);
            }

            TypedStmtKind::Return(None) | TypedStmtKind::Break | TypedStmtKind::Continue | TypedStmtKind::Needs(_) => {}
        }
    }

    fn propagate_function(&mut self, func: &mut TypedFunction) {
        self.scopes.push();
        for stmt in func.body.iter_mut() {
            self.propagate_stmt(stmt);
        }
        self.scopes.pop();
    }

    fn propagate_expr(&mut self, expr: &mut TypedExpr) {
        match &mut expr.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(const_val) = self.scopes.get(name) {
                    *expr = TypedExpr::new(const_val.kind.clone(), const_val.ty.clone(), expr.span);
                    self.stats.locals_propagated += 1;
                }
            }

            TypedExprKind::Binary { left, right, .. } => {
                self.propagate_expr(left);
                self.propagate_expr(right);
            }

            TypedExprKind::Unary { operand, .. } => {
                self.propagate_expr(operand);
            }

            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.propagate_expr(left);
                self.propagate_expr(right);
            }

            TypedExprKind::Call { callee, args } => {
                self.propagate_expr(callee);
                for arg in args.iter_mut() {
                    self.propagate_expr(arg);
                }
            }

            TypedExprKind::Assign { name, value } => {
                self.propagate_expr(value);
                self.scopes.invalidate(name);
            }

            TypedExprKind::Grouping(inner) => {
                self.propagate_expr(inner);
            }

            TypedExprKind::If { condition, then_branch, else_branch } => {
                self.propagate_expr(condition);
                self.propagate_expr(then_branch);
                self.propagate_expr(else_branch);
            }

            TypedExprKind::Lambda(inner) => {
                self.propagate_expr(inner);
            }

            TypedExprKind::LambdaInner { body, .. } => {
                self.scopes.push();
                for stmt in body.iter_mut() {
                    self.propagate_stmt(stmt);
                }
                self.scopes.pop();
            }

            TypedExprKind::Member { object, .. } => {
                self.propagate_expr(object);
            }

            TypedExprKind::ArrayLiteral { elements, .. } | TypedExprKind::VecLiteral { elements, .. } => {
                for elem in elements.iter_mut() {
                    self.propagate_expr(elem);
                }
            }

            TypedExprKind::Index { object, index } => {
                self.propagate_expr(object);
                self.propagate_expr(index);
            }

            TypedExprKind::IndexAssign { object, index, value } => {
                self.propagate_expr(object);
                self.propagate_expr(index);
                self.propagate_expr(value);
            }

            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.propagate_expr(s);
                }
                if let Some(e) = end {
                    self.propagate_expr(e);
                }
            }

            TypedExprKind::Slice { object, range } => {
                self.propagate_expr(object);
                self.propagate_expr(range);
            }

            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null => {}
        }
    }
}

impl Default for LocalConstantPropagator {
    fn default() -> Self {
        Self::new()
    }
}

impl OptimizationPass for LocalConstantPropagator {
    fn name(&self) -> &'static str {
        "local_const_prop"
    }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.scopes = ScopeStack::new();
        self.stats = OptimizationStats::new();

        for stmt in program.stmts.iter_mut() {
            self.propagate_stmt(stmt);
        }

        self.stats.clone()
    }
}
