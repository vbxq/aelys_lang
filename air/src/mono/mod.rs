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

    // Monomorphize generic enums
    monomorphize_enums(&mut program);

    program
}

/// Monomorphize generic enum definitions.
///
/// Scans all functions for `EnumInit`, `EnumTag`, and `EnumPayload` that reference
/// generic enums. Creates monomorphized copies of the enum definitions with concrete
/// types substituted in, and rewrites the enum_name references.
fn monomorphize_enums(program: &mut AirProgram) {
    // Collect generic enum indices
    let generic_enums: HashMap<String, usize> = program
        .enums
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.type_params.is_empty())
        .map(|(i, e)| (e.name.clone(), i))
        .collect();

    if generic_enums.is_empty() {
        return;
    }

    // Collect all (enum_name, type_args) pairs from EnumInit sites
    let mut enum_mono_requests: HashMap<(String, Vec<String>), Vec<AirType>> = HashMap::new();

    for func in &program.functions {
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign { rvalue, .. } = &stmt.kind {
                    collect_enum_mono_from_rvalue(
                        rvalue,
                        &generic_enums,
                        &program.enums,
                        func,
                        &mut enum_mono_requests,
                    );
                }
            }
        }
    }

    if enum_mono_requests.is_empty() {
        return;
    }

    // Create monomorphized enum definitions
    let mut mono_enum_names: HashMap<(String, Vec<String>), String> = HashMap::new();

    for (key, type_args) in &enum_mono_requests {
        let (enum_name, _) = key;
        let enum_idx = generic_enums[enum_name];
        let original = &program.enums[enum_idx];

        let type_str = type_args
            .iter()
            .map(substitute::type_to_string)
            .collect::<Vec<_>>()
            .join("_");
        let mangled_name = format!("__mono_{}_{}", enum_name, type_str);

        // Substitute type params in variant payload types
        let mono_variants: Vec<AirEnumVariant> = original
            .variants
            .iter()
            .map(|v| {
                let mono_payload: Vec<AirType> = v
                    .payload
                    .iter()
                    .map(|ty| substitute_enum_type(ty, &original.type_params, type_args))
                    .collect();
                AirEnumVariant {
                    name: v.name.clone(),
                    tag: v.tag,
                    payload: mono_payload,
                }
            })
            .collect();

        let mono_def = AirEnumDef {
            name: mangled_name.clone(),
            type_params: Vec::new(),
            variants: mono_variants,
            span: original.span,
        };

        program.enums.push(mono_def);
        mono_enum_names.insert(key.clone(), mangled_name);
    }

    // Rewrite enum_name references in all functions
    for func in &mut program.functions {
        for block in &mut func.blocks {
            for stmt in &mut block.stmts {
                rewrite_enum_refs_in_stmt(
                    stmt,
                    &generic_enums,
                    &program.enums,
                    &mono_enum_names,
                    &func.params.clone(),
                    &func.locals.clone(),
                );
            }
        }
        // Also rewrite local types that reference generic enums
        for local in &mut func.locals {
            if let AirType::Enum(ref name) = local.ty {
                if generic_enums.contains_key(name) {
                    // Try to find a mono'd version; if there's only one, use it
                    let mono_names: Vec<_> = mono_enum_names
                        .iter()
                        .filter(|((en, _), _)| en == name)
                        .map(|(_, mn)| mn.clone())
                        .collect();
                    if mono_names.len() == 1 {
                        local.ty = AirType::Enum(mono_names[0].clone());
                    }
                }
            }
        }
    }

    // Remove generic enum definitions (they've been replaced by mono'd versions)
    program.enums.retain(|e| e.type_params.is_empty());
}

fn substitute_enum_type(
    ty: &AirType,
    type_params: &[TypeParamId],
    type_args: &[AirType],
) -> AirType {
    match ty {
        AirType::Param(id) => {
            if let Some(idx) = type_params.iter().position(|p| p == id) {
                if let Some(replacement) = type_args.get(idx) {
                    return replacement.clone();
                }
            }
            ty.clone()
        }
        AirType::Ptr(inner) => AirType::Ptr(Box::new(substitute_enum_type(
            inner,
            type_params,
            type_args,
        ))),
        AirType::Array(inner, n) => AirType::Array(
            Box::new(substitute_enum_type(inner, type_params, type_args)),
            *n,
        ),
        AirType::Slice(inner) => AirType::Slice(Box::new(substitute_enum_type(
            inner,
            type_params,
            type_args,
        ))),
        AirType::FnPtr { params, ret, conv } => AirType::FnPtr {
            params: params
                .iter()
                .map(|p| substitute_enum_type(p, type_params, type_args))
                .collect(),
            ret: Box::new(substitute_enum_type(ret, type_params, type_args)),
            conv: *conv,
        },
        other => other.clone(),
    }
}

fn collect_enum_mono_from_rvalue(
    rvalue: &Rvalue,
    generic_enums: &HashMap<String, usize>,
    enum_defs: &[AirEnumDef],
    func: &AirFunction,
    requests: &mut HashMap<(String, Vec<String>), Vec<AirType>>,
) {
    match rvalue {
        Rvalue::EnumInit {
            enum_name, payload, ..
        } => {
            if let Some(&enum_idx) = generic_enums.get(enum_name) {
                let enum_def = &enum_defs[enum_idx];
                // Infer type args from payload operand types
                if let Some(type_args) = infer_enum_type_args(enum_def, rvalue, func) {
                    let key_strs: Vec<String> =
                        type_args.iter().map(substitute::type_to_string).collect();
                    let key = (enum_name.clone(), key_strs);
                    requests.entry(key).or_insert(type_args);
                } else if payload.is_empty() {
                    // Unit variant -- can't infer type args from payload.
                    // Type args will be inferred from other uses (e.g., from the local type).
                }
            }
        }
        Rvalue::EnumTag { enum_name, .. } | Rvalue::EnumPayload { enum_name, .. } => {
            // These will be handled by looking at the operand's type,
            // which should be a local with AirType::Enum("Option") etc.
            // The actual rewriting happens in the rewrite pass.
            if generic_enums.contains_key(enum_name) {
                // We'll handle these during rewriting
            }
        }
        _ => {}
    }
}

fn infer_enum_type_args(
    enum_def: &AirEnumDef,
    rvalue: &Rvalue,
    func: &AirFunction,
) -> Option<Vec<AirType>> {
    if let Rvalue::EnumInit {
        variant, payload, ..
    } = rvalue
    {
        let variant_def = enum_def.variants.iter().find(|v| v.name == *variant)?;

        let mut resolved: HashMap<u32, AirType> = HashMap::new();

        for (param_ty, operand) in variant_def.payload.iter().zip(payload.iter()) {
            let arg_ty = operand_type_from(operand, &func.params, &func.locals);
            unify_enum_param(param_ty, &arg_ty, &mut resolved);
        }

        let mut type_args = Vec::with_capacity(enum_def.type_params.len());
        for tp in &enum_def.type_params {
            type_args.push(resolved.get(&tp.0)?.clone());
        }
        Some(type_args)
    } else {
        None
    }
}

fn unify_enum_param(param_ty: &AirType, arg_ty: &AirType, resolved: &mut HashMap<u32, AirType>) {
    match param_ty {
        AirType::Param(id) => {
            resolved.entry(id.0).or_insert_with(|| arg_ty.clone());
        }
        AirType::Ptr(inner) => {
            if let AirType::Ptr(arg_inner) = arg_ty {
                unify_enum_param(inner, arg_inner, resolved);
            }
        }
        AirType::Array(inner, _) => {
            if let AirType::Array(arg_inner, _) = arg_ty {
                unify_enum_param(inner, arg_inner, resolved);
            }
        }
        AirType::Slice(inner) => {
            if let AirType::Slice(arg_inner) = arg_ty {
                unify_enum_param(inner, arg_inner, resolved);
            }
        }
        _ => {}
    }
}

fn rewrite_enum_refs_in_stmt(
    stmt: &mut AirStmt,
    generic_enums: &HashMap<String, usize>,
    enum_defs: &[AirEnumDef],
    mono_enum_names: &HashMap<(String, Vec<String>), String>,
    func_params: &[AirParam],
    func_locals: &[AirLocal],
) {
    if let AirStmtKind::Assign { rvalue, .. } = &mut stmt.kind {
        match rvalue {
            Rvalue::EnumInit {
                enum_name,
                variant,
                payload,
                ..
            } => {
                if let Some(&enum_idx) = generic_enums.get(enum_name.as_str()) {
                    let enum_def = &enum_defs[enum_idx];
                    let variant_def = enum_def.variants.iter().find(|v| v.name == *variant);
                    if let Some(vd) = variant_def {
                        let mut resolved: HashMap<u32, AirType> = HashMap::new();
                        for (param_ty, operand) in vd.payload.iter().zip(payload.iter()) {
                            let arg_ty = operand_type_from(operand, func_params, func_locals);
                            unify_enum_param(param_ty, &arg_ty, &mut resolved);
                        }
                        let type_args: Option<Vec<AirType>> = enum_def
                            .type_params
                            .iter()
                            .map(|tp| resolved.get(&tp.0).cloned())
                            .collect();
                        if let Some(type_args) = type_args {
                            let key_strs: Vec<String> =
                                type_args.iter().map(substitute::type_to_string).collect();
                            let key = (enum_name.clone(), key_strs);
                            if let Some(mangled) = mono_enum_names.get(&key) {
                                *enum_name = mangled.clone();
                            }
                        }
                    } else if payload.is_empty() {
                        // Unit variant -- try to find mono'd version from context.
                        // Look for any mono'd version of this enum (there should be one).
                        let mono_names: Vec<_> = mono_enum_names
                            .iter()
                            .filter(|((en, _), _)| en == enum_name.as_str())
                            .collect();
                        if mono_names.len() == 1 {
                            *enum_name = mono_names[0].1.clone();
                        }
                    }
                }
            }
            Rvalue::EnumTag { enum_name, .. } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    // Find the mono'd name from any available mono'd version
                    let mono_names: Vec<_> = mono_enum_names
                        .iter()
                        .filter(|((en, _), _)| en == enum_name.as_str())
                        .collect();
                    if mono_names.len() == 1 {
                        *enum_name = mono_names[0].1.clone();
                    }
                }
            }
            Rvalue::EnumPayload { enum_name, .. } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    let mono_names: Vec<_> = mono_enum_names
                        .iter()
                        .filter(|((en, _), _)| en == enum_name.as_str())
                        .collect();
                    if mono_names.len() == 1 {
                        *enum_name = mono_names[0].1.clone();
                    }
                }
            }
            _ => {}
        }
    }
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
