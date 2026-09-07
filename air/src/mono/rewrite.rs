use super::MonoContext;
use super::substitute::operand_type_from;
use crate::*;
use std::collections::{HashMap, HashSet};

impl MonoContext {
    pub(super) fn rewrite_call_sites(&self, program: &mut AirProgram) {
        if self.instantiated.is_empty() {
            return;
        }

        // collect return types of monomorphized functions so we can patch
        let mono_ret_types: HashMap<String, AirType> = self
            .instantiated
            .values()
            .filter_map(|mangled_name| {
                program
                    .functions
                    .iter()
                    .find(|f| f.name == *mangled_name)
                    .map(|f| (mangled_name.clone(), f.ret_ty.clone()))
            })
            .collect();

        // pre-collect generic function signatures for type inference during rewriting
        let generic_sigs: HashMap<String, (Vec<AirParam>, Vec<TypeParamId>)> = self
            .generic_functions
            .iter()
            .map(|(name, &idx)| {
                let f = &program.functions[idx];
                (name.clone(), (f.params.clone(), f.type_params.clone()))
            })
            .collect();
        let generic_names: HashSet<&str> = generic_sigs.keys().map(|s| s.as_str()).collect();

        for func in &mut program.functions {
            if !func.type_params.is_empty() {
                continue;
            }
            let caller_params = func.params.clone();
            let caller_locals = func.locals.clone();
            let mut local_type_patches: Vec<(LocalId, AirType)> = Vec::new();

            for block in &mut func.blocks {
                for stmt in &mut block.stmts {
                    self.rewrite_stmt(
                        stmt,
                        &caller_params,
                        &caller_locals,
                        &generic_sigs,
                        &generic_names,
                        &mono_ret_types,
                        &mut local_type_patches,
                    );
                }
                self.rewrite_terminator(
                    &mut block.terminator,
                    &caller_params,
                    &caller_locals,
                    &generic_sigs,
                    &generic_names,
                    &mono_ret_types,
                    &mut local_type_patches,
                );
            }

            // the actual return type of the monomorphized callee
            for (local_id, new_ty) in local_type_patches {
                if let Some(local) = func.locals.iter_mut().find(|l| l.id == local_id) {
                    local.ty = new_ty;
                }
            }
        }
    }

    fn rewrite_stmt(
        &self,
        stmt: &mut AirStmt,
        caller_params: &[AirParam],
        caller_locals: &[AirLocal],
        generic_sigs: &HashMap<String, (Vec<AirParam>, Vec<TypeParamId>)>,
        generic_names: &HashSet<&str>,
        mono_ret_types: &HashMap<String, AirType>,
        local_type_patches: &mut Vec<(LocalId, AirType)>,
    ) {
        match &mut stmt.kind {
            AirStmtKind::Assign {
                place,
                rvalue: Rvalue::Call { func: callee, args },
            } => {
                self.rewrite_callee(
                    callee,
                    args,
                    caller_params,
                    caller_locals,
                    generic_sigs,
                    generic_names,
                );
                // the monomorphized function's return type
                if let Place::Local(local_id) = place {
                    if let Callee::Named(name) = callee {
                        if let Some(ret_ty) = mono_ret_types.get(name.as_str()) {
                            local_type_patches.push((*local_id, ret_ty.clone()));
                        }
                    }
                }
            }
            AirStmtKind::CallVoid { func: callee, args } => {
                self.rewrite_callee(
                    callee,
                    args,
                    caller_params,
                    caller_locals,
                    generic_sigs,
                    generic_names,
                );
            }
            _ => {}
        }
    }

    fn rewrite_terminator(
        &self,
        term: &mut AirTerminator,
        caller_params: &[AirParam],
        caller_locals: &[AirLocal],
        generic_sigs: &HashMap<String, (Vec<AirParam>, Vec<TypeParamId>)>,
        generic_names: &HashSet<&str>,
        mono_ret_types: &HashMap<String, AirType>,
        local_type_patches: &mut Vec<(LocalId, AirType)>,
    ) {
        if let AirTerminator::Invoke {
            func: callee,
            args,
            ret,
            ..
        } = term
        {
            self.rewrite_callee(
                callee,
                args,
                caller_params,
                caller_locals,
                generic_sigs,
                generic_names,
            );
            if let Place::Local(local_id) = ret {
                if let Callee::Named(name) = callee {
                    if let Some(ret_ty) = mono_ret_types.get(name.as_str()) {
                        local_type_patches.push((*local_id, ret_ty.clone()));
                    }
                }
            }
        }
    }

    fn rewrite_callee(
        &self,
        callee: &mut Callee,
        args: &[Operand],
        caller_params: &[AirParam],
        caller_locals: &[AirLocal],
        generic_sigs: &HashMap<String, (Vec<AirParam>, Vec<TypeParamId>)>,
        generic_names: &HashSet<&str>,
    ) {
        if let Callee::Named(name) = callee
            && generic_names.contains(name.as_str())
        {
            if let Some((gen_params, gen_type_params)) = generic_sigs.get(name.as_str()) {
                if let Some(type_args) = self.infer_type_args_from_sig(
                    gen_params,
                    gen_type_params,
                    args,
                    caller_params,
                    caller_locals,
                ) {
                    let key = (name.clone(), self.type_args_key(&type_args));
                    if let Some(mangled) = self.instantiated.get(&key) {
                        *name = mangled.clone();
                    }
                }
            }
        }
    }

    fn infer_type_args_from_sig(
        &self,
        generic_params: &[AirParam],
        type_params: &[TypeParamId],
        args: &[Operand],
        caller_params: &[AirParam],
        caller_locals: &[AirLocal],
    ) -> Option<Vec<AirType>> {
        let mut resolved: HashMap<u32, AirType> = HashMap::new();

        for (param, arg) in generic_params.iter().zip(args.iter()) {
            let arg_ty = operand_type_from(arg, caller_params, caller_locals);
            self.unify_param(&param.ty, &arg_ty, type_params, &mut resolved);
        }

        let mut type_args = Vec::with_capacity(type_params.len());
        for tp in type_params {
            type_args.push(resolved.get(&tp.0)?.clone());
        }
        Some(type_args)
    }
}
