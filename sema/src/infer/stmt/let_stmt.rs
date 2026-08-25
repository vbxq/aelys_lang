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
        if self.nogc_fn_params.contains(name) {
            self.errors
                .push(crate::constraint::TypeError::nogc_param_shadowed(
                    name, span,
                ));
        }

        let mut typed_init = self.infer_expr(initializer);

        let declared_type = type_annotation
            .as_ref()
            .map(|ann| self.type_from_annotation(ann));

        let var_type = if let Some(decl) = &declared_type {
            // narrowing alone must never be the sole source of truth for a type
            self.try_narrow_literal(&mut typed_init, decl);

            if typed_init.ty != *decl && typed_init.ty.can_implicit_widen_to(decl) {
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

            self.constraints.push(Constraint::flows(
                typed_init.ty.clone(),
                decl.clone(),
                span,
                ConstraintReason::TypeAnnotation {
                    var_name: name.to_string(),
                },
            ));
            decl.clone()
        } else {
            if !mutable {
                Self::track_literal_init(&mut self.literal_init_vars, name, &typed_init);
            }
            typed_init.ty.clone()
        };

        self.check_rc_let_surface(name, mutable, &var_type, initializer, span);
        self.check_vec_let_surface(name, &var_type, span);

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

    fn check_rc_let_surface(
        &mut self,
        name: &str,
        mutable: bool,
        var_type: &crate::types::InferType,
        initializer: &Expr,
        span: Span,
    ) {
        use aelys_syntax::ExprKind;

        if var_type.is_rc() {
            if mutable {
                self.errors.push(
                    crate::constraint::TypeError::rc_out_of_surface(
                        format!("`Rc<T>` binding `{name}` cannot be `mut`: an Rc is single-assignment (not supported yet)"),
                        span,
                    ),
                );
            }
            let direct_init = matches!(
                &initializer.kind,
                ExprKind::EnumVariant { enum_name, variant, .. }
                    if enum_name == "Rc" && (variant == "new" || variant == "null")
            ) || matches!(
                initializer.kind,
                ExprKind::Identifier(_) | ExprKind::Member { .. }
            );
            if !direct_init {
                self.errors.push(
                    crate::constraint::TypeError::rc_out_of_surface(
                        format!(
                            "`Rc<T>` binding `{name}` must be initialized directly by `Rc::new(..)`, \
                             `Rc::null()`, by cloning another Rc binding, or by reading an Rc field; \
                             indirect/conditional init is not supported yet"
                        ),
                        span,
                    ),
                );
            }
        } else if var_type.contains_rc() || self.aggregate_embeds_rc_nominal(var_type) {
            self.errors
                .push(crate::constraint::TypeError::rc_out_of_surface(
                    format!(
                        "binding `{name}` has type `{var_type}` which embeds an `Rc<T>` in an \
                         aggregate value; Rc inside arrays/vecs/tuples/structs is not supported yet"
                    ),
                    span,
                ));
        }
    }

    // the construction sites already fail closed; this is the honest place to report, and it
    fn check_vec_let_surface(
        &mut self,
        name: &str,
        var_type: &crate::types::InferType,
        span: Span,
    ) {
        use crate::types::InferType;
        let holds = match var_type {
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.type_table.contains_vec_by_value(inner)
            }
            InferType::Tuple(elems) => elems
                .iter()
                .any(|e| self.type_table.contains_vec_by_value(e)),
            _ => false,
        };
        if holds {
            self.errors
                .push(crate::constraint::TypeError::vec_out_of_surface(
                    format!(
                        "binding `{name}` has type `{var_type}`, whose element holds a `Vec<T>` \
                         by value; a Vec inside a Vec/array is not supported yet (the buffer \
                         would be shared without a retain, the transitive Vec retain/release is \
                         not implemented)"
                    ),
                    span,
                ));
        }
    }

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
            TypedExprKind::Identifier(source_name) => {
                if let Some(lit) = literal_init_vars.get(source_name).cloned() {
                    literal_init_vars.insert(name.to_string(), lit);
                }
            }
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

    // which double-frees. a bare carrier binding is legal and tracked by carrier_locals
    fn aggregate_embeds_rc_nominal(&self, ty: &crate::types::InferType) -> bool {
        use crate::types::InferType;
        match ty {
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.type_table.contains_rc_nominal(inner)
            }
            InferType::Tuple(elems) => elems.iter().any(|e| self.type_table.contains_rc_nominal(e)),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum LiteralInit {
    Int(i64),
    Float(f64),
}
