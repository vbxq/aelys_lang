use super::TypeInference;
use crate::typed_ast::{
    TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedParam, TypedStmt,
    TypedStmtKind,
};
use crate::types::{InferType, ResolvedType};

impl TypeInference {
    /// Finalize types: walk the entire typed AST and convert any remaining unresolved `Var` to `Dynamic`
    ///
    /// after constraint solving+substitution, some type variables may remain unbound (for example empty arrays, unused generics, error recovery etc)
    /// without finalization these survive as InferType::Var(N) which AIR maps them to AirType::Void which causes miscompilation
    pub(super) fn finalize_stmts(&self, stmts: Vec<TypedStmt>) -> Vec<TypedStmt> {
        stmts.into_iter().map(|s| self.finalize_stmt(s)).collect()
    }

    fn finalize_stmt(&self, stmt: TypedStmt) -> TypedStmt {
        let kind = match stmt.kind {
            TypedStmtKind::Expression(expr) => {
                TypedStmtKind::Expression(self.finalize_expr(expr))
            }

            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                is_pub,
            } => TypedStmtKind::Let {
                name,
                mutable,
                initializer: self.finalize_expr(initializer),
                var_type: Self::finalize_type(var_type),
                is_pub,
            },

            TypedStmtKind::Block(stmts) => {
                TypedStmtKind::Block(self.finalize_stmts(stmts))
            }

            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => TypedStmtKind::If {
                condition: self.finalize_expr(condition),
                then_branch: Box::new(self.finalize_stmt(*then_branch)),
                else_branch: else_branch.map(|e| Box::new(self.finalize_stmt(*e))),
            },

            TypedStmtKind::While { condition, body } => TypedStmtKind::While {
                condition: self.finalize_expr(condition),
                body: Box::new(self.finalize_stmt(*body)),
            },

            TypedStmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => TypedStmtKind::For {
                iterator,
                start: self.finalize_expr(start),
                end: self.finalize_expr(end),
                inclusive,
                step: Box::new((*step).map(|s| self.finalize_expr(s))),
                body: Box::new(self.finalize_stmt(*body)),
            },

            TypedStmtKind::ForEach {
                iterator,
                iterable,
                elem_type,
                body,
            } => TypedStmtKind::ForEach {
                iterator,
                iterable: self.finalize_expr(iterable),
                elem_type: Self::finalize_type(elem_type),
                body: Box::new(self.finalize_stmt(*body)),
            },

            TypedStmtKind::Return(expr) => {
                TypedStmtKind::Return(expr.map(|e| self.finalize_expr(e)))
            }

            TypedStmtKind::Break => TypedStmtKind::Break,
            TypedStmtKind::Continue => TypedStmtKind::Continue,

            TypedStmtKind::Function(func) => {
                TypedStmtKind::Function(self.finalize_function(func))
            }

            TypedStmtKind::Needs(needs) => TypedStmtKind::Needs(needs),

            TypedStmtKind::StructDecl {
                name,
                type_params,
                fields,
            } => TypedStmtKind::StructDecl {
                name,
                type_params,
                fields: fields
                    .into_iter()
                    .map(|(n, ty)| (n, Self::finalize_type(ty)))
                    .collect(),
            },
        };

        TypedStmt {
            kind,
            span: stmt.span,
        }
    }

    fn finalize_expr(&self, expr: TypedExpr) -> TypedExpr {
        let kind = match expr.kind {
            TypedExprKind::Int(n) => TypedExprKind::Int(n),
            TypedExprKind::Float(f) => TypedExprKind::Float(f),
            TypedExprKind::Bool(b) => TypedExprKind::Bool(b),
            TypedExprKind::String(s) => TypedExprKind::String(s),
            TypedExprKind::Null => TypedExprKind::Null,

            TypedExprKind::FmtString(parts) => TypedExprKind::FmtString(
                parts
                    .into_iter()
                    .map(|p| match p {
                        TypedFmtStringPart::Literal(s) => TypedFmtStringPart::Literal(s),
                        TypedFmtStringPart::Expr(e) => {
                            TypedFmtStringPart::Expr(Box::new(self.finalize_expr(*e)))
                        }
                        TypedFmtStringPart::Placeholder => TypedFmtStringPart::Placeholder,
                    })
                    .collect(),
            ),

            TypedExprKind::Identifier(name) => TypedExprKind::Identifier(name),

            TypedExprKind::Binary { left, op, right } => TypedExprKind::Binary {
                left: Box::new(self.finalize_expr(*left)),
                op,
                right: Box::new(self.finalize_expr(*right)),
            },

            TypedExprKind::Unary { op, operand } => TypedExprKind::Unary {
                op,
                operand: Box::new(self.finalize_expr(*operand)),
            },

            TypedExprKind::And { left, right } => TypedExprKind::And {
                left: Box::new(self.finalize_expr(*left)),
                right: Box::new(self.finalize_expr(*right)),
            },

            TypedExprKind::Or { left, right } => TypedExprKind::Or {
                left: Box::new(self.finalize_expr(*left)),
                right: Box::new(self.finalize_expr(*right)),
            },

            TypedExprKind::Call { callee, args } => TypedExprKind::Call {
                callee: Box::new(self.finalize_expr(*callee)),
                args: args.into_iter().map(|a| self.finalize_expr(a)).collect(),
            },

            TypedExprKind::Assign { name, value } => TypedExprKind::Assign {
                name,
                value: Box::new(self.finalize_expr(*value)),
            },

            TypedExprKind::Grouping(inner) => {
                TypedExprKind::Grouping(Box::new(self.finalize_expr(*inner)))
            }

            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => TypedExprKind::If {
                condition: Box::new(self.finalize_expr(*condition)),
                then_branch: Box::new(self.finalize_expr(*then_branch)),
                else_branch: Box::new(self.finalize_expr(*else_branch)),
            },

            TypedExprKind::Lambda(inner) => {
                TypedExprKind::Lambda(Box::new(self.finalize_expr(*inner)))
            }

            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => TypedExprKind::LambdaInner {
                params: params
                    .into_iter()
                    .map(|p| TypedParam {
                        ty: Self::finalize_type(p.ty),
                        ..p
                    })
                    .collect(),
                return_type: Self::finalize_type(return_type),
                body: self.finalize_stmts(body),
                captures: captures
                    .into_iter()
                    .map(|(name, ty)| (name, Self::finalize_type(ty)))
                    .collect(),
            },

            TypedExprKind::Member { object, member } => TypedExprKind::Member {
                object: Box::new(self.finalize_expr(*object)),
                member,
            },

            TypedExprKind::ArrayLiteral { elements } => TypedExprKind::ArrayLiteral {
                elements: elements
                    .into_iter()
                    .map(|e| self.finalize_expr(e))
                    .collect(),
            },

            TypedExprKind::ArraySized { size, fill_value } => TypedExprKind::ArraySized {
                size: Box::new(self.finalize_expr(*size)),
                fill_value: fill_value.map(|fv| Box::new(self.finalize_expr(*fv))),
            },

            TypedExprKind::VecLiteral {
                element_type: _,
                elements,
            } => {
                // regenerate element_type from the finalized outer Vec type,
                // since the original snapshot may be stale after substitution.
                let finalized_outer = Self::finalize_type(expr.ty.clone());
                let new_elem_type = match &finalized_outer {
                    InferType::Vec(inner) => Some(ResolvedType::from_infer_type(inner)),
                    _ => None,
                };
                TypedExprKind::VecLiteral {
                    element_type: new_elem_type,
                    elements: elements
                        .into_iter()
                        .map(|e| self.finalize_expr(e))
                        .collect(),
                }
            }

            TypedExprKind::Index { object, index } => TypedExprKind::Index {
                object: Box::new(self.finalize_expr(*object)),
                index: Box::new(self.finalize_expr(*index)),
            },

            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => TypedExprKind::IndexAssign {
                object: Box::new(self.finalize_expr(*object)),
                index: Box::new(self.finalize_expr(*index)),
                value: Box::new(self.finalize_expr(*value)),
            },

            TypedExprKind::Range {
                start,
                end,
                inclusive,
            } => TypedExprKind::Range {
                start: start.map(|s| Box::new(self.finalize_expr(*s))),
                end: end.map(|e| Box::new(self.finalize_expr(*e))),
                inclusive,
            },

            TypedExprKind::Slice { object, range } => TypedExprKind::Slice {
                object: Box::new(self.finalize_expr(*object)),
                range: Box::new(self.finalize_expr(*range)),
            },

            TypedExprKind::StructLiteral { name, fields } => TypedExprKind::StructLiteral {
                name,
                fields: fields
                    .into_iter()
                    .map(|(n, v)| (n, Box::new(self.finalize_expr(*v))))
                    .collect(),
            },

            TypedExprKind::Cast { expr, target } => TypedExprKind::Cast {
                expr: Box::new(self.finalize_expr(*expr)),
                target: Self::finalize_type(target),
            },
        };

        TypedExpr {
            kind,
            ty: Self::finalize_type(expr.ty),
            span: expr.span,
        }
    }

    fn finalize_function(&self, func: TypedFunction) -> TypedFunction {
        TypedFunction {
            params: func
                .params
                .into_iter()
                .map(|p| TypedParam {
                    ty: Self::finalize_type(p.ty),
                    ..p
                })
                .collect(),
            return_type: Self::finalize_type(func.return_type),
            body: self.finalize_stmts(func.body),
            captures: func
                .captures
                .into_iter()
                .map(|(name, ty)| (name, Self::finalize_type(ty)))
                .collect(),
            ..func
        }
    }

    /// Convert any remaining `Var` to `Dynamic` in a type.
    /// Recurses into compound types (Array, Vec, Function, Tuple).
    fn finalize_type(ty: InferType) -> InferType {
        match ty {
            InferType::Var(_) => InferType::Dynamic,
            InferType::Array(inner, len) => {
                InferType::Array(Box::new(Self::finalize_type(*inner)), len)
            }
            InferType::Vec(inner) => InferType::Vec(Box::new(Self::finalize_type(*inner))),
            InferType::Function { params, ret } => InferType::Function {
                params: params.into_iter().map(Self::finalize_type).collect(),
                ret: Box::new(Self::finalize_type(*ret)),
            },
            InferType::Tuple(elems) => {
                InferType::Tuple(elems.into_iter().map(Self::finalize_type).collect())
            }
            // concrete types pass through unchanged
            other => other,
        }
    }
}
