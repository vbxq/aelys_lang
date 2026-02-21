use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExprKind, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span, TypeAnnotation};

fn int_fits(value: i64, ty: &InferType) -> bool {
    match ty {
        InferType::I8 => i8::try_from(value).is_ok(),
        InferType::I16 => i16::try_from(value).is_ok(),
        InferType::I32 => i32::try_from(value).is_ok(),
        InferType::I64 => true,
        InferType::U8 => u8::try_from(value).is_ok(),
        InferType::U16 => u16::try_from(value).is_ok(),
        InferType::U32 => u32::try_from(value).is_ok(),
        InferType::U64 => value >= 0,
        _ => false,
    }
}

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
        let mut typed_init = self.infer_expr(initializer);

        let declared_type = type_annotation
            .as_ref()
            .map(|ann| self.type_from_annotation(ann));

        let var_type = if let Some(decl) = &declared_type {
            if let TypedExprKind::Int(value) = &typed_init.kind
                && decl.is_integer()
                && *decl != InferType::I64
            {
                if int_fits(*value, decl) {
                    typed_init.ty = decl.clone();
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: decl.clone(),
                            found: InferType::I64,
                        },
                        span: typed_init.span,
                        reason: ConstraintReason::IntLiteralOverflow {
                            value: *value,
                            target: decl.clone(),
                        },
                    });
                }
            } else {
                self.constraints.push(Constraint::equal(
                    typed_init.ty.clone(),
                    decl.clone(),
                    span,
                    ConstraintReason::TypeAnnotation {
                        var_name: name.to_string(),
                    },
                ));
            }
            decl.clone()
        } else {
            typed_init.ty.clone()
        };

        self.env.define_local(name.to_string(), var_type.clone());

        TypedStmtKind::Let {
            name: name.to_string(),
            mutable,
            initializer: typed_init,
            var_type,
            is_pub,
        }
    }
}
