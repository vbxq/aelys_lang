use super::GlobalConstantPropagator;
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedPattern, TypedStmt, TypedStmtKind};

impl GlobalConstantPropagator {
    pub(super) fn substitute_constants(&mut self, expr: &mut TypedExpr) {
        match &mut expr.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(c) = self.constants.get(name) {
                    let ty = if expr.ty.is_integer() && c.ty.is_integer() {
                        expr.ty.clone()
                    } else {
                        c.ty.clone()
                    };
                    *expr = TypedExpr::new(c.kind.clone(), ty, expr.span);
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
            TypedExprKind::ArraySized {
                size, fill_value, ..
            } => {
                self.substitute_constants(size);
                if let Some(fv) = fill_value {
                    self.substitute_constants(fv);
                }
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
            TypedExprKind::FieldAssign {
                object,
                value,
                ..
            } => {
                self.substitute_constants(object);
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
            TypedExprKind::Reference { operand, .. } => self.substitute_constants(operand),
            TypedExprKind::Deref(operand) => self.substitute_constants(operand),
            TypedExprKind::DerefAssign { target, value } => {
                self.substitute_constants(target);
                self.substitute_constants(value);
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                        self.substitute_constants(e);
                    }
                }
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.substitute_constants(value);
                }
            }
            TypedExprKind::Cast { expr, .. } => {
                self.substitute_constants(expr);
            }
            TypedExprKind::Block { stmts, tail } => {
                for stmt in stmts {
                    self.substitute_in_stmt(stmt);
                }
                self.substitute_constants(tail);
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.substitute_constants(scrutinee);
                for arm in arms {
                    let shadowed: Vec<(String, aelys_sema::TypedExpr)> =
                        if let TypedPattern::Variant { bindings, .. } = &arm.pattern {
                            bindings
                                .iter()
                                .filter_map(|(name, _)| {
                                    self.constants.remove(name).map(|v| (name.clone(), v))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                    self.substitute_constants(&mut arm.body);
                    for (name, val) in shadowed {
                        self.constants.insert(name, val);
                    }
                }
            }
            TypedExprKind::ResultAssert { scrutinee, .. } => {
                self.substitute_constants(scrutinee);
            }
            TypedExprKind::EnumVariant { args, .. } => {
                for arg in args {
                    self.substitute_constants(arg);
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
                self.substitute_in_scoped_stmts(stmts);
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
                iterator,
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
                let shadowed = self.constants.remove(iterator);
                self.substitute_in_stmt(body);
                if let Some(val) = shadowed {
                    self.constants.insert(iterator.clone(), val);
                }
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                body,
                ..
            } => {
                self.substitute_constants(iterable);
                let shadowed = self.constants.remove(iterator);
                self.substitute_in_stmt(body);
                if let Some(val) = shadowed {
                    self.constants.insert(iterator.clone(), val);
                }
            }
            TypedStmtKind::Return(Some(expr)) => self.substitute_constants(expr),
            TypedStmtKind::Function(func) => self.substitute_in_function(func),
            TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }

    fn substitute_in_function(&mut self, func: &mut TypedFunction) {
        let shadowed_by_params: Vec<(String, aelys_sema::TypedExpr)> = func
            .params
            .iter()
            .filter_map(|p| self.constants.remove(&p.name).map(|v| (p.name.clone(), v)))
            .collect();
        self.substitute_in_scoped_stmts(&mut func.body);
        for (name, val) in shadowed_by_params {
            self.constants.insert(name, val);
        }
    }

    /// Process a statement list, removing any global constant whose name is
    /// shadowed by a local `let` so that subsequent statements in the same
    /// scope see the local binding rather than the global one.
    fn substitute_in_scoped_stmts(&mut self, stmts: &mut Vec<TypedStmt>) {
        let mut shadowed: Vec<(String, aelys_sema::TypedExpr)> = Vec::new();
        for stmt in stmts.iter_mut() {
            self.substitute_in_stmt(stmt);
            if let TypedStmtKind::Let { name, .. } = &stmt.kind {
                if let Some(val) = self.constants.remove(name) {
                    shadowed.push((name.clone(), val));
                }
            }
        }
        for (name, val) in shadowed {
            self.constants.insert(name, val);
        }
    }
}
