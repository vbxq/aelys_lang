mod rewrite;
mod substitute;

use crate::*;
use std::collections::{HashMap, HashSet};
use substitute::operand_type_from;

pub fn monomorphize(mut program: AirProgram) -> AirProgram {
    let mut ctx = MonoContext::new(&program);
    ctx.collect_mono_requests(&program);
    ctx.instantiate(&mut program);
    ctx.rewrite_call_sites(&mut program);
    program.functions.retain(|f| f.type_params.is_empty());
    program
}

pub(super) struct MonoContext {
    pub(super) generic_functions: HashMap<String, usize>,
    pub(super) requests: Vec<MonoRequest>,
    pub(super) instantiated: HashMap<(String, Vec<String>), String>,
    pub(super) next_function_id: u32,
}

pub(super) struct MonoRequest {
    pub(super) function_name: String,
    pub(super) type_args: Vec<AirType>,
}

impl MonoContext {
    fn new(program: &AirProgram) -> Self {
        let generic_functions: HashMap<String, usize> = program
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.type_params.is_empty())
            .map(|(i, f)| (f.name.clone(), i))
            .collect();

        Self {
            generic_functions,
            requests: Vec::new(),
            instantiated: HashMap::new(),
            next_function_id: program.functions.len() as u32,
        }
    }

    fn collect_mono_requests(&mut self, program: &AirProgram) {
        let generic_names: HashSet<String> = self.generic_functions.keys().cloned().collect();

        for func in &program.functions {
            if func.type_params.is_empty() {
                self.collect_from_function(func, program, &generic_names);
            }
        }
    }

    fn collect_from_function(
        &mut self,
        func: &AirFunction,
        program: &AirProgram,
        generic_names: &HashSet<String>,
    ) {
        for block in &func.blocks {
            for stmt in &block.stmts {
                self.collect_from_stmt(stmt, func, program, generic_names);
            }
            self.collect_from_terminator(&block.terminator, func, program, generic_names);
        }
    }

    fn collect_from_stmt(
        &mut self,
        stmt: &AirStmt,
        caller: &AirFunction,
        program: &AirProgram,
        generic_names: &HashSet<String>,
    ) {
        match &stmt.kind {
            AirStmtKind::Assign {
                rvalue: Rvalue::Call { func: callee, args },
                ..
            } => {
                self.try_collect(callee, args, caller, program, generic_names);
            }
            AirStmtKind::CallVoid { func: callee, args } => {
                self.try_collect(callee, args, caller, program, generic_names);
            }
            _ => {}
        }
    }

    fn collect_from_terminator(
        &mut self,
        term: &AirTerminator,
        caller: &AirFunction,
        program: &AirProgram,
        generic_names: &HashSet<String>,
    ) {
        if let AirTerminator::Invoke {
            func: callee, args, ..
        } = term
        {
            self.try_collect(callee, args, caller, program, generic_names);
        }
    }

    fn try_collect(
        &mut self,
        callee: &Callee,
        args: &[Operand],
        caller: &AirFunction,
        program: &AirProgram,
        generic_names: &HashSet<String>,
    ) {
        let name = match callee {
            Callee::Named(n) if generic_names.contains(n) => n,
            _ => return,
        };

        let func_idx = self.generic_functions[name];
        let generic_func = &program.functions[func_idx];

        if let Some(type_args) = self.infer_type_args(generic_func, args, caller) {
            let key = (name.clone(), self.type_args_key(&type_args));
            if !self.instantiated.contains_key(&key) {
                self.requests.push(MonoRequest {
                    function_name: name.clone(),
                    type_args,
                });
            }
        }
    }

    fn infer_type_args(
        &self,
        generic_func: &AirFunction,
        args: &[Operand],
        caller: &AirFunction,
    ) -> Option<Vec<AirType>> {
        let mut resolved: HashMap<u32, AirType> = HashMap::new();

        for (param, arg) in generic_func.params.iter().zip(args.iter()) {
            let arg_ty = operand_type_from(arg, &caller.params, &caller.locals);
            self.unify_param(&param.ty, &arg_ty, &mut resolved);
        }

        let mut type_args = Vec::with_capacity(generic_func.type_params.len());
        for tp in &generic_func.type_params {
            type_args.push(resolved.get(&tp.0)?.clone());
        }
        Some(type_args)
    }

    pub(super) fn unify_param(
        &self,
        param_ty: &AirType,
        arg_ty: &AirType,
        resolved: &mut HashMap<u32, AirType>,
    ) {
        match param_ty {
            AirType::Param(id) => {
                resolved.entry(id.0).or_insert_with(|| arg_ty.clone());
            }
            AirType::Ptr(inner) => {
                if let AirType::Ptr(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, resolved);
                }
            }
            AirType::Array(inner, _) => {
                if let AirType::Array(arg_inner, _) = arg_ty {
                    self.unify_param(inner, arg_inner, resolved);
                }
            }
            AirType::Slice(inner) => {
                if let AirType::Slice(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, resolved);
                }
            }
            AirType::FnPtr { params, ret, .. } => {
                if let AirType::FnPtr {
                    params: arg_params,
                    ret: arg_ret,
                    ..
                } = arg_ty
                {
                    for (p, a) in params.iter().zip(arg_params.iter()) {
                        self.unify_param(p, a, resolved);
                    }
                    self.unify_param(ret, arg_ret, resolved);
                }
            }
            _ => {}
        }
    }

    pub(super) fn type_args_key(&self, types: &[AirType]) -> Vec<String> {
        types.iter().map(substitute::type_to_string).collect()
    }

    pub(super) fn mangle_name(&self, name: &str, type_args: &[AirType]) -> String {
        if type_args.is_empty() {
            return name.to_string();
        }
        let type_str = type_args
            .iter()
            .map(substitute::type_to_string)
            .collect::<Vec<_>>()
            .join("_");
        format!("__mono_{}_{}", name, type_str)
    }

    fn instantiate(&mut self, program: &mut AirProgram) {
        let mut new_functions = Vec::new();
        let mut mono_instances = Vec::new();

        for request in &self.requests {
            let key = (
                request.function_name.clone(),
                self.type_args_key(&request.type_args),
            );

            if self.instantiated.contains_key(&key) {
                continue;
            }

            let func_idx = self.generic_functions[&request.function_name];
            let original_func = &program.functions[func_idx];
            let original_id = original_func.id;
            let new_id = FunctionId(self.next_function_id);
            self.next_function_id += 1;

            let mangled_name = self.mangle_name(&request.function_name, &request.type_args);
            let saved_type_params = original_func.type_params.clone();
            let mut new_func = original_func.clone();
            new_func.id = new_id;
            new_func.name = mangled_name.clone();
            new_func.type_params = Vec::new();

            substitute::substitute_types_in_function(
                &mut new_func,
                &saved_type_params,
                &request.type_args,
            );

            new_functions.push(new_func);
            self.instantiated.insert(key, mangled_name);

            mono_instances.push(MonoInstance {
                original: original_id,
                type_args: request.type_args.clone(),
                result: new_id,
            });
        }

        program.functions.extend(new_functions);
        program.mono_instances.extend(mono_instances);
    }
}
