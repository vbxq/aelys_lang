use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};
use std::collections::HashMap;

impl TypeInference {
    /// Replace type parameter placeholders (`Struct("T")` where T is not a real struct) with fresh type variables
    ///
    /// this way each call site gets its own instantiation.
    ///
    /// same params within a single function type share the same fresh var (so (T) -> T becomes (Var(N)) -> Var(N))
    fn instantiate_type_params(&mut self, ty: &InferType) -> InferType {
        let mut mapping: HashMap<String, InferType> = HashMap::new();
        self.instantiate_inner(ty, &mut mapping)
    }

    fn instantiate_inner(
        &mut self,
        ty: &InferType,
        mapping: &mut HashMap<String, InferType>,
    ) -> InferType {
        match ty {
            InferType::Struct(name) if !self.type_table.has_struct(name) => mapping
                .entry(name.clone())
                .or_insert_with(|| self.type_gen.fresh())
                .clone(),
            InferType::Function { params, ret } => {
                let new_params = params
                    .iter()
                    .map(|p| self.instantiate_inner(p, mapping))
                    .collect();
                let new_ret = Box::new(self.instantiate_inner(ret, mapping));
                InferType::Function {
                    params: new_params,
                    ret: new_ret,
                }
            }
            InferType::Array(inner, len) => {
                InferType::Array(Box::new(self.instantiate_inner(inner, mapping)), *len)
            }
            InferType::Vec(inner) => {
                InferType::Vec(Box::new(self.instantiate_inner(inner, mapping)))
            }
            InferType::Tuple(elems) => InferType::Tuple(
                elems
                    .iter()
                    .map(|e| self.instantiate_inner(e, mapping))
                    .collect(),
            ),
            InferType::Enum(name, type_args) if !type_args.is_empty() => InferType::Enum(
                name.clone(),
                type_args
                    .iter()
                    .map(|a| self.instantiate_inner(a, mapping))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

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
            // instance generic type params so each call site gets fresh vars instead of the shared Struct("T") placeholders
            let callee_ty = self.instantiate_type_params(&typed_callee.ty);

            if let InferType::Function { params, .. } = &callee_ty
                && params.len() == typed_args.len()
            {
                // try to narrow numeric literals to match parameter types
                for (arg, param_ty) in typed_args.iter_mut().zip(params.iter()) {
                    self.try_narrow_literal(arg, param_ty);
                }

                // implicit numeric widening for non-literal arguments
                for (arg, param_ty) in typed_args.iter_mut().zip(params.iter()) {
                    if arg.ty != *param_ty && arg.ty.can_implicit_widen_to(param_ty) {
                        let span = arg.span;
                        let original = std::mem::replace(
                            arg,
                            TypedExpr {
                                kind: TypedExprKind::Null,
                                ty: InferType::Null,
                                span,
                            },
                        );
                        *arg = TypedExpr {
                            kind: TypedExprKind::Cast {
                                expr: Box::new(original),
                                target: param_ty.clone(),
                            },
                            ty: param_ty.clone(),
                            span,
                        };
                    }
                }
            }

            // When the callee has a known function type, extract the return type
            // directly so that downstream expressions (e.g., match) can inspect
            // it before constraint solving runs. For unknown callee types, fall
            // back to a fresh type variable resolved via constraints.
            let ret = if let InferType::Function { ret: fn_ret, .. } = &callee_ty {
                *fn_ret.clone()
            } else {
                self.type_gen.fresh()
            };

            let arg_types: Vec<InferType> = typed_args.iter().map(|a| a.ty.clone()).collect();
            let expected_fn_type = InferType::Function {
                params: arg_types,
                ret: Box::new(ret.clone()),
            };

            self.constraints.push(Constraint::equal(
                callee_ty,
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
