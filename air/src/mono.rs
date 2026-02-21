use crate::*;
use std::collections::{HashMap, HashSet};

type MangledMap<'a> = HashMap<&'a str, Vec<(&'a (String, Vec<String>), &'a String)>>;

pub fn monomorphize(mut program: AirProgram) -> AirProgram {
    let mut ctx = MonoContext::new(&program);
    ctx.collect_mono_requests(&program);
    ctx.instantiate(&mut program);
    ctx.rewrite_call_sites(&mut program);
    program.functions.retain(|f| f.type_params.is_empty());
    program
}

struct MonoContext {
    generic_functions: HashMap<String, usize>,
    requests: Vec<MonoRequest>,
    instantiated: HashMap<(String, Vec<String>), String>,
    next_function_id: u32,
}

struct MonoRequest {
    function_name: String,
    type_args: Vec<AirType>,
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
            let arg_ty = self.operand_type(arg, caller);
            self.unify_param(&param.ty, &arg_ty, &mut resolved);
        }

        let mut type_args = Vec::with_capacity(generic_func.type_params.len());
        for tp in &generic_func.type_params {
            type_args.push(resolved.get(&tp.0)?.clone());
        }
        Some(type_args)
    }

    fn unify_param(
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

    fn operand_type(&self, operand: &Operand, caller: &AirFunction) -> AirType {
        match operand {
            Operand::Const(c) => match c {
                AirConst::IntLiteral(_) => AirType::I64,
                AirConst::Int(_, size) => match size {
                    AirIntSize::I8 => AirType::I8,
                    AirIntSize::I16 => AirType::I16,
                    AirIntSize::I32 => AirType::I32,
                    AirIntSize::I64 => AirType::I64,
                    AirIntSize::U8 => AirType::U8,
                    AirIntSize::U16 => AirType::U16,
                    AirIntSize::U32 => AirType::U32,
                    AirIntSize::U64 => AirType::U64,
                },
                AirConst::Float(_, size) => match size {
                    AirFloatSize::F32 => AirType::F32,
                    AirFloatSize::F64 => AirType::F64,
                },
                AirConst::Bool(_) => AirType::Bool,
                AirConst::Str(_) => AirType::Str,
                AirConst::Null => AirType::Void,
                AirConst::ZeroInit(ty) | AirConst::Undef(ty) => ty.clone(),
            },
            Operand::Copy(id) | Operand::Move(id) => {
                caller.params.iter().find(|p| p.id == *id).map(|p| p.ty.clone())
                    .or_else(|| caller.locals.iter().find(|l| l.id == *id).map(|l| l.ty.clone()))
                    .unwrap_or(AirType::I64)
            }
        }
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

            self.substitute_types_in_function(
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

    fn rewrite_call_sites(&self, program: &mut AirProgram) {
        let name_to_mangled: MangledMap = {
            let mut map: HashMap<&str, Vec<_>> = HashMap::new();
            for (key, mangled) in &self.instantiated {
                map.entry(key.0.as_str()).or_default().push((key, mangled));
            }
            map
        };

        if name_to_mangled.is_empty() {
            return;
        }

        for func in &mut program.functions {
            if !func.type_params.is_empty() {
                continue;
            }
            for block in &mut func.blocks {
                for stmt in &mut block.stmts {
                    self.rewrite_stmt(stmt, &name_to_mangled);
                }
                self.rewrite_terminator(&mut block.terminator, &name_to_mangled);
            }
        }
    }

    fn rewrite_stmt(&self, stmt: &mut AirStmt, name_map: &MangledMap) {
        match &mut stmt.kind {
            AirStmtKind::Assign {
                rvalue: Rvalue::Call { func: callee, .. },
                ..
            } => {
                self.rewrite_callee(callee, name_map);
            }
            AirStmtKind::CallVoid { func: callee, .. } => {
                self.rewrite_callee(callee, name_map);
            }
            _ => {}
        }
    }

    fn rewrite_terminator(&self, term: &mut AirTerminator, name_map: &MangledMap) {
        if let AirTerminator::Invoke { func: callee, .. } = term {
            self.rewrite_callee(callee, name_map);
        }
    }

    fn rewrite_callee(&self, callee: &mut Callee, name_map: &MangledMap) {
        if let Callee::Named(name) = callee
            && let Some(entries) = name_map.get(name.as_str())
            && let Some((_, mangled)) = entries.first()
        {
            *name = (*mangled).clone();
        }
    }

    fn type_args_key(&self, types: &[AirType]) -> Vec<String> {
        types.iter().map(type_to_string).collect()
    }

    fn mangle_name(&self, name: &str, type_args: &[AirType]) -> String {
        if type_args.is_empty() {
            return name.to_string();
        }
        let type_str = type_args
            .iter()
            .map(type_to_string)
            .collect::<Vec<_>>()
            .join("_");
        format!("__mono_{}_{}", name, type_str)
    }

    fn substitute_types_in_function(
        &self,
        func: &mut AirFunction,
        type_params: &[TypeParamId],
        type_args: &[AirType],
    ) {
        for param in &mut func.params {
            substitute_type(&mut param.ty, type_params, type_args);
        }
        substitute_type(&mut func.ret_ty, type_params, type_args);

        for local in &mut func.locals {
            substitute_type(&mut local.ty, type_params, type_args);
        }

        for block in &mut func.blocks {
            for stmt in &mut block.stmts {
                substitute_stmt(stmt, type_params, type_args);
            }
            substitute_terminator(&mut block.terminator, type_params, type_args);
        }
    }
}

fn type_to_string(ty: &AirType) -> String {
    match ty {
        AirType::I8 => "i8".to_string(),
        AirType::I16 => "i16".to_string(),
        AirType::I32 => "i32".to_string(),
        AirType::I64 => "i64".to_string(),
        AirType::U8 => "u8".to_string(),
        AirType::U16 => "u16".to_string(),
        AirType::U32 => "u32".to_string(),
        AirType::U64 => "u64".to_string(),
        AirType::F32 => "f32".to_string(),
        AirType::F64 => "f64".to_string(),
        AirType::Bool => "bool".to_string(),
        AirType::Str => "str".to_string(),
        AirType::Ptr(inner) => format!("ptr_{}", type_to_string(inner)),
        AirType::Struct(name) => name.clone(),
        AirType::Array(inner, size) => format!("array_{}_{}", type_to_string(inner), size),
        AirType::Slice(inner) => format!("slice_{}", type_to_string(inner)),
        AirType::FnPtr { .. } => "fnptr".to_string(),
        AirType::Param(id) => format!("param_{}", id.0),
        AirType::Void => "void".to_string(),
    }
}

fn substitute_type(ty: &mut AirType, type_params: &[TypeParamId], type_args: &[AirType]) {
    match ty {
        AirType::Param(id) => {
            if let Some(idx) = type_params.iter().position(|p| p == id)
                && let Some(replacement) = type_args.get(idx)
            {
                *ty = replacement.clone();
            }
        }
        AirType::Ptr(inner) => substitute_type(inner, type_params, type_args),
        AirType::Array(inner, _) => substitute_type(inner, type_params, type_args),
        AirType::Slice(inner) => substitute_type(inner, type_params, type_args),
        AirType::FnPtr { params, ret, .. } => {
            for p in params {
                substitute_type(p, type_params, type_args);
            }
            substitute_type(ret, type_params, type_args);
        }
        _ => {}
    }
}

fn substitute_stmt(stmt: &mut AirStmt, type_params: &[TypeParamId], type_args: &[AirType]) {
    match &mut stmt.kind {
        AirStmtKind::Assign { rvalue, .. } => {
            substitute_rvalue(rvalue, type_params, type_args);
        }
        AirStmtKind::GcAlloc { ty, .. } | AirStmtKind::Alloc { ty, .. } => {
            substitute_type(ty, type_params, type_args);
        }
        _ => {}
    }
}

fn substitute_rvalue(rvalue: &mut Rvalue, type_params: &[TypeParamId], type_args: &[AirType]) {
    match rvalue {
        Rvalue::Cast { from, to, .. } => {
            substitute_type(from, type_params, type_args);
            substitute_type(to, type_params, type_args);
        }
        Rvalue::StructInit { name, .. } => {
            for tp in type_params {
                if let Some(replacement) = type_args.get(tp.0 as usize) {
                    let mangled_suffix = type_to_string(replacement);
                    if name.contains("__mono_") {
                        continue;
                    }
                    *name = format!("__mono_{}_{}", name, mangled_suffix);
                }
            }
        }
        _ => {}
    }
}

fn substitute_terminator(
    term: &mut AirTerminator,
    type_params: &[TypeParamId],
    type_args: &[AirType],
) {
    if let AirTerminator::Invoke { func: callee, .. } = term {
        substitute_callee(callee, type_params, type_args);
    }
}

fn substitute_callee(_callee: &mut Callee, _type_params: &[TypeParamId], _type_args: &[AirType]) {
    // Callee rewriting happens in the separate rewrite_call_sites pass
    // after all instances are known. No per-function substitution needed.
}
