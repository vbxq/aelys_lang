use super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedParam};
use crate::types::InferType;
use aelys_syntax::{Parameter, Span, Stmt, TypeAnnotation};

impl TypeInference {
    /// Infer lambda (anonymous function)
    pub(super) fn infer_lambda(
        &mut self,
        params: &[Parameter],
        return_type_ann: Option<&TypeAnnotation>,
        body: &[Stmt],
        span: Span,
    ) -> TypedExpr {
        let closure_env = self.env.for_closure();

        let mut typed_params = Vec::with_capacity(params.len());
        for p in params {
            let ty = match &p.type_annotation {
                Some(ann) => self.type_from_annotation(ann),
                None => self.type_gen.fresh(),
            };
            typed_params.push(TypedParam {
                name: p.name.clone(),
                mutable: p.mutable,
                ty,
                span: p.span,
            });
        }

        let return_type = match return_type_ann {
            Some(ann) => self.type_from_annotation(ann),
            None => self.type_gen.fresh(),
        };

        let saved_env = std::mem::replace(&mut self.env, closure_env);

        for param in &typed_params {
            self.env.define_local(param.name.clone(), param.ty.clone());
        }

        self.push_return_type(return_type.clone());

        let typed_stmts = if body.is_empty() {
            vec![]
        } else {
            let mut stmts = Vec::new();
            for stmt in &body[..body.len() - 1] {
                stmts.push(self.infer_stmt(stmt));
            }

            let last_stmt = &body[body.len() - 1];
            let typed_last = self.infer_stmt_with_implicit_return(last_stmt, &return_type);
            stmts.push(typed_last);

            stmts
        };

        let captures = self.collect_captures_from_stmts(&typed_stmts, &typed_params);

        self.pop_return_type();
        self.env = saved_env;

        let param_types: Vec<InferType> = typed_params.iter().map(|p| p.ty.clone()).collect();
        let fn_type = InferType::Function {
            params: param_types,
            ret: Box::new(return_type.clone()),
        };

        TypedExpr {
            kind: TypedExprKind::LambdaInner {
                params: typed_params,
                return_type,
                body: typed_stmts,
                captures,
            },
            ty: fn_type,
            span,
        }
    }
}
