use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

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
    pub(super) fn infer_call_expr(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_callee = self.infer_expr(callee);
        let mut typed_args: Vec<TypedExpr> = args.iter().map(|a| self.infer_expr(a)).collect();

        let ret_type = if matches!(typed_callee.ty, InferType::Dynamic) {
            InferType::Dynamic
        } else {
            if let InferType::Function { params, .. } = &typed_callee.ty {
                if params.len() == typed_args.len() {
                    for (arg, param_ty) in typed_args.iter_mut().zip(params.iter()) {
                        if let TypedExprKind::Int(value) = &arg.kind {
                            if param_ty.is_integer() && *param_ty != InferType::I64 {
                                if int_fits(*value, param_ty) {
                                    arg.ty = param_ty.clone();
                                } else {
                                    self.errors.push(TypeError {
                                        kind: TypeErrorKind::Mismatch {
                                            expected: param_ty.clone(),
                                            found: InferType::I64,
                                        },
                                        span: arg.span,
                                        reason: ConstraintReason::IntLiteralOverflow {
                                            value: *value,
                                            target: param_ty.clone(),
                                        },
                                    });
                                }
                            }
                        }
                    }
                }
            }

            let ret = self.type_gen.fresh();

            let arg_types: Vec<InferType> = typed_args.iter().map(|a| a.ty.clone()).collect();
            let expected_fn_type = InferType::Function {
                params: arg_types,
                ret: Box::new(ret.clone()),
            };

            self.constraints.push(Constraint::equal(
                typed_callee.ty.clone(),
                expected_fn_type,
                span,
                ConstraintReason::Other("function call".to_string()),
            ));

            ret
        };

        (
            TypedExprKind::Call {
                callee: Box::new(typed_callee),
                args: typed_args,
            },
            ret_type,
        )
    }
}
