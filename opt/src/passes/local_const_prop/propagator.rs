use super::scope::ScopeStack;
use crate::passes::{ConstantFolder, OptimizationPass, OptimizationStats};
use aelys_sema::{
    TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedProgram, TypedStmt,
    TypedStmtKind,
};

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
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                ..
            } => {
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

            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
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
                let mut assigned = Vec::new();
                Self::collect_assigned_vars(body, &mut assigned);
                for name in &assigned {
                    self.scopes.invalidate(name);
                }
                self.propagate_expr(condition);
                self.scopes.push();
                self.propagate_stmt(body);
                self.scopes.pop();
            }

            TypedStmtKind::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.propagate_expr(start);
                self.propagate_expr(end);
                if let Some(s) = &mut **step {
                    self.propagate_expr(s);
                }
                let mut assigned = Vec::new();
                Self::collect_assigned_vars(body, &mut assigned);
                for name in &assigned {
                    self.scopes.invalidate(name);
                }
                self.scopes.push();
                self.propagate_stmt(body);
                self.scopes.pop();
            }

            TypedStmtKind::ForEach { iterable, body, .. } => {
                self.propagate_expr(iterable);
                let mut assigned = Vec::new();
                Self::collect_assigned_vars(body, &mut assigned);
                for name in &assigned {
                    self.scopes.invalidate(name);
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

            TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. } => {}
        }
    }

    fn collect_assigned_vars(stmt: &TypedStmt, out: &mut Vec<String>) {
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => Self::collect_assigned_vars_expr(expr, out),
            TypedStmtKind::Block(stmts) => {
                for s in stmts {
                    Self::collect_assigned_vars(s, out);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::collect_assigned_vars_expr(condition, out);
                Self::collect_assigned_vars(then_branch, out);
                if let Some(e) = else_branch {
                    Self::collect_assigned_vars(e, out);
                }
            }
            TypedStmtKind::While { condition, body } => {
                Self::collect_assigned_vars_expr(condition, out);
                Self::collect_assigned_vars(body, out);
            }
            TypedStmtKind::For { body, .. } => Self::collect_assigned_vars(body, out),
            TypedStmtKind::ForEach { body, .. } => Self::collect_assigned_vars(body, out),
            TypedStmtKind::Return(Some(expr)) => Self::collect_assigned_vars_expr(expr, out),
            _ => {}
        }
    }

    fn collect_assigned_vars_expr(expr: &TypedExpr, out: &mut Vec<String>) {
        match &expr.kind {
            TypedExprKind::Assign { name, value } => {
                out.push(name.clone());
                Self::collect_assigned_vars_expr(value, out);
            }
            TypedExprKind::Binary { left, right, .. }
            | TypedExprKind::And { left, right }
            | TypedExprKind::Or { left, right } => {
                Self::collect_assigned_vars_expr(left, out);
                Self::collect_assigned_vars_expr(right, out);
            }
            TypedExprKind::Call { callee, args } => {
                Self::collect_assigned_vars_expr(callee, out);
                for a in args {
                    Self::collect_assigned_vars_expr(a, out);
                }
            }
            TypedExprKind::Unary { operand, .. }
            | TypedExprKind::Grouping(operand)
            | TypedExprKind::Lambda(operand)
            | TypedExprKind::Cast { expr: operand, .. } => {
                Self::collect_assigned_vars_expr(operand, out);
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::collect_assigned_vars_expr(condition, out);
                Self::collect_assigned_vars_expr(then_branch, out);
                Self::collect_assigned_vars_expr(else_branch, out);
            }
            TypedExprKind::Index { object, index }
            | TypedExprKind::Slice {
                object,
                range: index,
            } => {
                Self::collect_assigned_vars_expr(object, out);
                Self::collect_assigned_vars_expr(index, out);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                Self::collect_assigned_vars_expr(object, out);
                Self::collect_assigned_vars_expr(index, out);
                Self::collect_assigned_vars_expr(value, out);
            }
            TypedExprKind::Member { object, .. } => {
                Self::collect_assigned_vars_expr(object, out);
            }
            TypedExprKind::ArrayLiteral { elements, .. }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for e in elements {
                    Self::collect_assigned_vars_expr(e, out);
                }
            }
            TypedExprKind::ArraySized { size, .. } => {
                Self::collect_assigned_vars_expr(size, out);
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, val) in fields {
                    Self::collect_assigned_vars_expr(val, out);
                }
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    Self::collect_assigned_vars_expr(s, out);
                }
                if let Some(e) = end {
                    Self::collect_assigned_vars_expr(e, out);
                }
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let TypedFmtStringPart::Expr(e) = part {
                        Self::collect_assigned_vars_expr(e, out);
                    }
                }
            }
            TypedExprKind::LambdaInner { body, .. } => {
                for s in body {
                    Self::collect_assigned_vars(s, out);
                }
            }
            _ => {}
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
                    let ty = if expr.ty.is_integer() && const_val.ty.is_integer() {
                        expr.ty.clone()
                    } else {
                        const_val.ty.clone()
                    };
                    *expr = TypedExpr::new(const_val.kind.clone(), ty, expr.span);
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

            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
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

            TypedExprKind::ArrayLiteral { elements, .. }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for elem in elements.iter_mut() {
                    self.propagate_expr(elem);
                }
            }

            TypedExprKind::ArraySized { size, .. } => {
                self.propagate_expr(size);
            }

            TypedExprKind::Index { object, index } => {
                self.propagate_expr(object);
                self.propagate_expr(index);
            }

            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
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

            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                        self.propagate_expr(e);
                    }
                }
            }

            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields.iter_mut() {
                    self.propagate_expr(value);
                }
            }
            TypedExprKind::Cast { expr, .. } => {
                self.propagate_expr(expr);
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
