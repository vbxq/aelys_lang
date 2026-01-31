use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};
use aelys_syntax::Span;
use std::collections::HashMap;

pub struct InlineExpander {
    _reserved: (),
}

impl InlineExpander {
    pub fn new() -> Self {
        Self { _reserved: () }
    }

    pub fn expand_call(
        &mut self,
        func: &TypedFunction,
        args: &[TypedExpr],
        call_span: Span,
    ) -> Option<TypedExpr> {
        // don't inline if arity doesn't match
        if func.params.len() != args.len() {
            return None;
        }

        let param_map: HashMap<String, TypedExpr> = func
            .params
            .iter()
            .zip(args.iter())
            .map(|(p, a)| (p.name.clone(), a.clone()))
            .collect();

        self.try_simple_inline(&func.body, &param_map, call_span)
    }

    fn try_simple_inline(
        &self,
        body: &[TypedStmt],
        params: &HashMap<String, TypedExpr>,
        span: Span,
    ) -> Option<TypedExpr> {
        // only inline truly trivial bodies: single return or expression with no let bindings
        if body.len() != 1 { return None; }

        match &body[0].kind {
            TypedStmtKind::Return(Some(expr)) => {
                if self.expr_has_only_params_and_literals(expr, params) {
                    Some(self.substitute_expr(expr, params, span))
                } else {
                    None
                }
            }
            TypedStmtKind::Expression(expr) => {
                if self.expr_has_only_params_and_literals(expr, params) {
                    Some(self.substitute_expr(expr, params, span))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn expr_has_only_params_and_literals(&self, expr: &TypedExpr, params: &HashMap<String, TypedExpr>) -> bool {
        match &expr.kind {
            TypedExprKind::Int(_) | TypedExprKind::Float(_) |
            TypedExprKind::Bool(_) | TypedExprKind::String(_) | TypedExprKind::Null => true,

            TypedExprKind::Identifier(name) => params.contains_key(name),

            TypedExprKind::Binary { left, right, .. } => {
                self.expr_has_only_params_and_literals(left, params) &&
                self.expr_has_only_params_and_literals(right, params)
            }
            TypedExprKind::Unary { operand, .. } => {
                self.expr_has_only_params_and_literals(operand, params)
            }
            TypedExprKind::Grouping(inner) => {
                self.expr_has_only_params_and_literals(inner, params)
            }
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.expr_has_only_params_and_literals(left, params) &&
                self.expr_has_only_params_and_literals(right, params)
            }
            TypedExprKind::If { condition, then_branch, else_branch } => {
                self.expr_has_only_params_and_literals(condition, params) &&
                self.expr_has_only_params_and_literals(then_branch, params) &&
                self.expr_has_only_params_and_literals(else_branch, params)
            }
            // anything else (calls, arrays, etc.) - don't inline
            _ => false,
        }
    }

    fn substitute_expr(
        &self,
        expr: &TypedExpr,
        params: &HashMap<String, TypedExpr>,
        span: Span,
    ) -> TypedExpr {
        let kind = match &expr.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(replacement) = params.get(name) {
                    return replacement.clone();
                }
                TypedExprKind::Identifier(name.clone())
            }

            TypedExprKind::Binary { left, op, right } => TypedExprKind::Binary {
                left: Box::new(self.substitute_expr(left, params, span)),
                op: *op,
                right: Box::new(self.substitute_expr(right, params, span)),
            },

            TypedExprKind::Unary { op, operand } => TypedExprKind::Unary {
                op: *op,
                operand: Box::new(self.substitute_expr(operand, params, span)),
            },

            TypedExprKind::And { left, right } => TypedExprKind::And {
                left: Box::new(self.substitute_expr(left, params, span)),
                right: Box::new(self.substitute_expr(right, params, span)),
            },

            TypedExprKind::Or { left, right } => TypedExprKind::Or {
                left: Box::new(self.substitute_expr(left, params, span)),
                right: Box::new(self.substitute_expr(right, params, span)),
            },

            TypedExprKind::Call { callee, args } => TypedExprKind::Call {
                callee: Box::new(self.substitute_expr(callee, params, span)),
                args: args.iter().map(|a| self.substitute_expr(a, params, span)).collect(),
            },

            TypedExprKind::Grouping(inner) => {
                TypedExprKind::Grouping(Box::new(self.substitute_expr(inner, params, span)))
            }

            TypedExprKind::If { condition, then_branch, else_branch } => TypedExprKind::If {
                condition: Box::new(self.substitute_expr(condition, params, span)),
                then_branch: Box::new(self.substitute_expr(then_branch, params, span)),
                else_branch: Box::new(self.substitute_expr(else_branch, params, span)),
            },

            TypedExprKind::Assign { name, value } => TypedExprKind::Assign {
                name: name.clone(),
                value: Box::new(self.substitute_expr(value, params, span)),
            },

            TypedExprKind::Member { object, member } => TypedExprKind::Member {
                object: Box::new(self.substitute_expr(object, params, span)),
                member: member.clone(),
            },

            TypedExprKind::ArrayLiteral { element_type, elements } => TypedExprKind::ArrayLiteral {
                element_type: element_type.clone(),
                elements: elements.iter().map(|e| self.substitute_expr(e, params, span)).collect(),
            },

            TypedExprKind::VecLiteral { element_type, elements } => TypedExprKind::VecLiteral {
                element_type: element_type.clone(),
                elements: elements.iter().map(|e| self.substitute_expr(e, params, span)).collect(),
            },

            TypedExprKind::ArraySized { element_type, size } => TypedExprKind::ArraySized {
                element_type: element_type.clone(),
                size: Box::new(self.substitute_expr(size, params, span)),
            },

            TypedExprKind::Index { object, index } => TypedExprKind::Index {
                object: Box::new(self.substitute_expr(object, params, span)),
                index: Box::new(self.substitute_expr(index, params, span)),
            },

            TypedExprKind::IndexAssign { object, index, value } => TypedExprKind::IndexAssign {
                object: Box::new(self.substitute_expr(object, params, span)),
                index: Box::new(self.substitute_expr(index, params, span)),
                value: Box::new(self.substitute_expr(value, params, span)),
            },

            TypedExprKind::Range { start, end, inclusive } => TypedExprKind::Range {
                start: start.as_ref().map(|s| Box::new(self.substitute_expr(s, params, span))),
                end: end.as_ref().map(|e| Box::new(self.substitute_expr(e, params, span))),
                inclusive: *inclusive,
            },

            TypedExprKind::Slice { object, range } => TypedExprKind::Slice {
                object: Box::new(self.substitute_expr(object, params, span)),
                range: Box::new(self.substitute_expr(range, params, span)),
            },

            // lambdas need special care to avoid capturing the wrong variables
            TypedExprKind::Lambda(inner) => {
                TypedExprKind::Lambda(Box::new(self.substitute_expr(inner, params, span)))
            }

            TypedExprKind::LambdaInner { params: lparams, return_type, body, captures } => {
                // don't substitute params that shadow the outer ones
                let mut filtered = params.clone();
                for p in lparams {
                    filtered.remove(&p.name);
                }
                TypedExprKind::LambdaInner {
                    params: lparams.clone(),
                    return_type: return_type.clone(),
                    body: body.iter().map(|s| self.substitute_stmt(s, &filtered, span)).collect(),
                    captures: captures.clone(),
                }
            }

            TypedExprKind::FmtString(parts) => TypedExprKind::FmtString(
                parts.iter().map(|p| match p {
                    aelys_sema::TypedFmtStringPart::Literal(s) => aelys_sema::TypedFmtStringPart::Literal(s.clone()),
                    aelys_sema::TypedFmtStringPart::Expr(e) => aelys_sema::TypedFmtStringPart::Expr(Box::new(self.substitute_expr(e, params, span))),
                    aelys_sema::TypedFmtStringPart::Placeholder => aelys_sema::TypedFmtStringPart::Placeholder,
                }).collect()
            ),

            // literals pass through unchanged
            TypedExprKind::Int(n) => TypedExprKind::Int(*n),
            TypedExprKind::Float(f) => TypedExprKind::Float(*f),
            TypedExprKind::Bool(b) => TypedExprKind::Bool(*b),
            TypedExprKind::String(s) => TypedExprKind::String(s.clone()),
            TypedExprKind::Null => TypedExprKind::Null,
        };

        TypedExpr::new(kind, expr.ty.clone(), span)
    }

    fn substitute_stmt(
        &self,
        stmt: &TypedStmt,
        params: &HashMap<String, TypedExpr>,
        span: Span,
    ) -> TypedStmt {
        let kind = match &stmt.kind {
            TypedStmtKind::Expression(e) => {
                TypedStmtKind::Expression(self.substitute_expr(e, params, span))
            }
            TypedStmtKind::Let { name, mutable, initializer, var_type, is_pub } => {
                TypedStmtKind::Let {
                    name: name.clone(),
                    mutable: *mutable,
                    initializer: self.substitute_expr(initializer, params, span),
                    var_type: var_type.clone(),
                    is_pub: *is_pub,
                }
            }
            TypedStmtKind::Block(stmts) => {
                TypedStmtKind::Block(
                    stmts.iter().map(|s| self.substitute_stmt(s, params, span)).collect()
                )
            }
            TypedStmtKind::If { condition, then_branch, else_branch } => TypedStmtKind::If {
                condition: self.substitute_expr(condition, params, span),
                then_branch: Box::new(self.substitute_stmt(then_branch, params, span)),
                else_branch: else_branch.as_ref().map(|e| Box::new(self.substitute_stmt(e, params, span))),
            },
            TypedStmtKind::While { condition, body } => TypedStmtKind::While {
                condition: self.substitute_expr(condition, params, span),
                body: Box::new(self.substitute_stmt(body, params, span)),
            },
            TypedStmtKind::For { iterator, start, end, inclusive, step, body } => TypedStmtKind::For {
                iterator: iterator.clone(),
                start: self.substitute_expr(start, params, span),
                end: self.substitute_expr(end, params, span),
                inclusive: *inclusive,
                step: step.as_ref().map(|s| self.substitute_expr(s, params, span)),
                body: Box::new(self.substitute_stmt(body, params, span)),
            },
            TypedStmtKind::Return(e) => {
                TypedStmtKind::Return(e.as_ref().map(|ex| self.substitute_expr(ex, params, span)))
            }
            other => other.clone(),
        };

        TypedStmt::new(kind, stmt.span)
    }
}

impl Default for InlineExpander {
    fn default() -> Self { Self::new() }
}
