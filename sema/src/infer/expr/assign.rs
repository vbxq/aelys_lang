use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorSuggestion};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_assign_expr(
        &mut self,
        name: &str,
        value: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let mut typed_value = self.infer_expr(value);

        if let Some(var_type) = self.env.lookup(name).cloned() {
            // check mutability, reject assignment to immutable variables
            if !self.env.is_mutable(name) {
                let binding_span = self.env.lookup_binding_span(name);
                let suggestion = binding_span.map(|bs| {
                    let insert_offset = bs.start + 4; // skip "let "
                    let insert_span =
                        Span::new(insert_offset, insert_offset, bs.line, bs.column + 4);
                    TypeErrorSuggestion {
                        message: "make the binding mutable".to_string(),
                        span: insert_span,
                        new_text: "mut ".to_string(),
                    }
                });
                self.errors.push(TypeError::assign_to_immutable(
                    name.to_string(),
                    span,
                    binding_span,
                    suggestion,
                ));
            }

            self.coerce_dyn(&mut typed_value, &var_type);
            self.try_narrow_literal(&mut typed_value, &var_type);

            if typed_value.ty != var_type && typed_value.ty.can_implicit_widen_to(&var_type) {
                let vspan = typed_value.span;
                let original = std::mem::replace(
                    &mut typed_value,
                    TypedExpr {
                        kind: TypedExprKind::Null,
                        ty: InferType::Null,
                        span: vspan,
                    },
                );
                typed_value = TypedExpr {
                    kind: TypedExprKind::Cast {
                        expr: Box::new(original),
                        target: var_type.clone(),
                    },
                    ty: var_type.clone(),
                    span: vspan,
                };
            }

            self.constraints.push(Constraint::flows(
                typed_value.ty.clone(),
                var_type.clone(),
                span,
                ConstraintReason::Assignment {
                    var_name: name.to_string(),
                },
            ));

            if var_type == InferType::String && self.env.lookup_local(name).is_some() {
                match self.append_in_place(name, typed_value) {
                    Ok(kind) => return (kind, InferType::Null),
                    Err(value) => typed_value = value,
                }
            }

            (
                TypedExprKind::Assign {
                    name: name.to_string(),
                    value: Box::new(typed_value),
                },
                InferType::Null,
            )
        } else {
            match self.module_bound_elsewhere(name) {
                Some((module, binding)) => {
                    self.errors.push(TypeError::module_bound_elsewhere(
                        name.to_string(),
                        module,
                        binding,
                        span,
                    ));
                }
                None => self
                    .errors
                    .push(TypeError::undefined_variable(name.to_string(), span)),
            }

            // register the variable with dynamic type to prevent repeated "undefined variable" errors for each subsequent use
            self.env.define_local(name.to_string(), InferType::Dynamic);

            (
                TypedExprKind::Assign {
                    name: name.to_string(),
                    value: Box::new(typed_value),
                },
                InferType::Null,
            )
        }
    }

    // `s = s + e` grows `s` in place; e runs first, so it must not be able to write `s`
    fn append_in_place(&self, name: &str, value: TypedExpr) -> Result<TypedExprKind, TypedExpr> {
        let TypedExprKind::Binary {
            left,
            op: aelys_syntax::BinaryOp::Add,
            right,
        } = &value.kind
        else {
            return Err(value);
        };
        let names_target = matches!(&left.kind, TypedExprKind::Identifier(n) if n == name);
        if !names_target || !right.ty.is_str() || !self.cannot_write_locals(right) {
            return Err(value);
        }
        let TypedExprKind::Binary { left, right, .. } = value.kind else {
            unreachable!("matched above");
        };
        Ok(TypedExprKind::EnumVariant {
            enum_name: "string".to_string(),
            variant: "push_str".to_string(),
            tag: 0,
            args: vec![*left, *right],
        })
    }

    fn cannot_write_locals(&self, e: &TypedExpr) -> bool {
        let go = |e: &TypedExpr| self.cannot_write_locals(e);
        match &e.kind {
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::Char(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null
            | TypedExprKind::Identifier(_) => true,
            TypedExprKind::FmtString(parts) => parts.iter().all(|p| match p {
                crate::typed_ast::TypedFmtStringPart::Expr(e) => go(e),
                _ => true,
            }),
            TypedExprKind::Grouping(inner)
            | TypedExprKind::Unary { operand: inner, .. }
            | TypedExprKind::Cast { expr: inner, .. }
            | TypedExprKind::Member { object: inner, .. } => go(inner),
            TypedExprKind::Binary { left, right, .. }
            | TypedExprKind::And { left, right }
            | TypedExprKind::Or { left, right }
            | TypedExprKind::Index {
                object: left,
                index: right,
            }
            | TypedExprKind::Slice {
                object: left,
                range: right,
            } => go(left) && go(right),
            TypedExprKind::Range { start, end, .. } => {
                start.as_deref().is_none_or(go) && end.as_deref().is_none_or(go)
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                args,
                ..
            } => !(enum_name == "string" && variant.starts_with("push")) && args.iter().all(go),
            TypedExprKind::Call { callee, args } => {
                matches!(&callee.kind, TypedExprKind::Identifier(f) if self.env.lookup_local(f).is_none())
                    && args.iter().all(go)
            }
            _ => false,
        }
    }
}
