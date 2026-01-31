use super::TypeInference;
use crate::typed_ast::{TypedFunction, TypedParam};
use crate::types::InferType;
use aelys_syntax::Function;

impl TypeInference {
    /// Infer function type
    pub(super) fn infer_function(&mut self, func: &Function) -> TypedFunction {
        let fn_signature = self.env.lookup_function(&func.name).cloned();

        let (sig_params, sig_ret) = match fn_signature.as_deref() {
            Some(InferType::Function { params, ret }) => {
                (Some(params.clone()), Some((**ret).clone()))
            }
            _ => (None, None),
        };

        let mut typed_params = Vec::with_capacity(func.params.len());
        for (i, p) in func.params.iter().enumerate() {
            let ty = sig_params
                .as_ref()
                .and_then(|ps| ps.get(i).cloned())
                .or_else(|| p.type_annotation.as_ref().map(|ann| self.type_from_annotation(ann)))
                .unwrap_or_else(|| self.type_gen.fresh());

            typed_params.push(TypedParam {
                name: p.name.clone(),
                ty,
                span: p.span,
            });
        }

        let return_type = sig_ret.or_else(|| {
            func.return_type.as_ref().map(|ann| self.type_from_annotation(ann))
        }).unwrap_or_else(|| self.type_gen.fresh());

        let mut func_env = self.env.for_closure();
        func_env.set_current_function(Some(func.name.clone()));

        for param in &typed_params {
            func_env.define_local(param.name.clone(), param.ty.clone());
        }

        let saved_env = std::mem::replace(&mut self.env, func_env);

        self.collect_signatures(&func.body, &func.name);

        self.push_return_type(return_type.clone());

        let typed_body = if func.body.is_empty() {
            vec![]
        } else {
            let mut stmts: Vec<_> = func.body[..func.body.len() - 1]
                .iter()
                .map(|s| self.infer_stmt(s))
                .collect();

            let last_stmt = &func.body[func.body.len() - 1];
            let typed_last = self.infer_stmt_with_implicit_return(last_stmt, &return_type);
            stmts.push(typed_last);

            stmts
        };

        let captures = self.collect_captures_from_stmts(&typed_body, &typed_params);

        self.pop_return_type();
        self.env = saved_env;

        TypedFunction {
            name: func.name.clone(),
            params: typed_params,
            return_type,
            body: typed_body,
            decorators: func.decorators.clone(),
            is_pub: func.is_pub,
            span: func.span,
            captures,
        }
    }
}
