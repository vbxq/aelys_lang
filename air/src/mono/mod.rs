mod rewrite;
pub(crate) mod substitute;

use crate::passes::vec_surface::SurfaceErrorKind;
use crate::*;
use aelys_common::Fault;
use std::collections::{HashMap, HashSet};
use substitute::operand_type_from;

const MONO_ROUNDS: usize = 64;
const ENUM_MONO_LIMIT: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonoErrorKind {
    VecSurface,
    NoDefinition,
    Mono,
}

#[derive(Debug, Clone)]
pub struct MonoError {
    pub kind: MonoErrorKind,
    pub fault: Fault,
    pub message: String,
}

impl MonoError {
    fn mono(fault: Fault, message: String) -> Self {
        MonoError {
            kind: MonoErrorKind::Mono,
            fault,
            message,
        }
    }

    fn vec_surface(message: String) -> Self {
        MonoError {
            kind: MonoErrorKind::VecSurface,
            fault: Fault::Unsupported,
            message,
        }
    }
}

pub fn monomorphize(mut program: AirProgram) -> Result<AirProgram, Vec<MonoError>> {
    let mut ctx = MonoContext::new(&program);
    // an instance body can itself call a generic, so collection reruns until it adds nothing
    for _ in 0..MONO_ROUNDS {
        ctx.requests.clear();
        ctx.collect_mono_requests(&program);
        if !ctx.errors.is_empty() {
            return Err(ctx.errors);
        }
        if ctx.requests.is_empty() {
            break;
        }
        let surface_errors = ctx.instantiate(&mut program);
        if !surface_errors.is_empty() {
            return Err(surface_errors);
        }
    }
    ctx.rewrite_call_sites(&mut program);
    program.functions.retain(|f| f.type_params.is_empty());

    let uninstantiated = ctx.uninstantiated_call_sites(&program);
    if !uninstantiated.is_empty() {
        return Err(uninstantiated);
    }

    // monomorphize generic enums
    let errors = monomorphize_enums(&mut program);
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(program)
}

/// monomorphize generic enum definitions.
fn monomorphize_enums(program: &mut AirProgram) -> Vec<MonoError> {
    // collect generic enum indices
    let generic_enums: HashMap<String, usize> = program
        .enums
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.type_params.is_empty())
        .map(|(i, e)| (e.name.clone(), i))
        .collect();

    if generic_enums.is_empty() {
        return Vec::new();
    }

    let mut errors: Vec<MonoError> = Vec::new();
    let mut uses = EnumUses {
        generic: &generic_enums,
        wanted: Vec::new(),
        seen: HashSet::new(),
        bare: Vec::new(),
    };

    for func in &program.functions {
        for param in &func.params {
            uses.ty(&param.ty);
        }
        uses.ty(&func.ret_ty);
        for local in &func.locals {
            uses.ty(&local.ty);
        }
        for block in &func.blocks {
            for stmt in &block.stmts {
                uses.stmt(stmt);
            }
            uses.terminator(&block.terminator);
        }
    }
    for global in &program.globals {
        uses.ty(&global.ty);
        if let Some(init) = &global.init {
            uses.konst(init);
        }
    }
    for def in &program.structs {
        for field in &def.fields {
            uses.ty(&field.ty);
        }
    }
    for def in &program.enums {
        if !def.type_params.is_empty() {
            continue;
        }
        for variant in &def.variants {
            for payload in &variant.payload {
                uses.ty(payload);
            }
        }
    }

    let mut instances: Vec<AirEnumDef> = Vec::new();
    let mut cursor = 0usize;
    while cursor < uses.wanted.len() {
        if instances.len() >= ENUM_MONO_LIMIT {
            errors.push(MonoError::mono(
                Fault::Compiler,
                format!(
                    "generic enum instantiation did not terminate after {ENUM_MONO_LIMIT} \
                     instances; an enum whose payload names itself under its own type argument \
                     would do this and sema is expected to have refused it"
                ),
            ));
            break;
        }
        let request = uses.wanted[cursor].clone();
        cursor += 1;
        let enum_idx = generic_enums[&request.name];
        let original = &program.enums[enum_idx];
        if original.type_params.len() != request.args.len() {
            errors.push(MonoError::mono(
                Fault::Compiler,
                format!(
                    "enum `{}` takes {} type arguments and the lowered program asks for {}",
                    request.name,
                    original.type_params.len(),
                    request.args.len()
                ),
            ));
            continue;
        }
        let variants: Vec<AirEnumVariant> = original
            .variants
            .iter()
            .map(|v| AirEnumVariant {
                name: v.name.clone(),
                tag: v.tag,
                payload: v
                    .payload
                    .iter()
                    .map(|ty| substitute_type_params(ty, &original.type_params, &request.args))
                    .collect(),
            })
            .collect();
        for variant in &variants {
            for payload in &variant.payload {
                uses.ty(payload);
            }
        }
        instances.push(AirEnumDef {
            name: request.symbol(),
            type_params: Vec::new(),
            variants,
            span: original.span,
        });
    }

    for name in uses.bare {
        errors.push(MonoError::mono(
            Fault::Compiler,
            format!(
                "`{name}` is a generic enum and the lowered program names it with no type \
                 arguments, so no instance answers the reference"
            ),
        ));
    }

    program.enums.extend(instances);
    // remove generic enum definitions (they've been replaced by mono'd versions)
    program.enums.retain(|e| e.type_params.is_empty());

    errors
}

struct EnumUses<'a> {
    generic: &'a HashMap<String, usize>,
    wanted: Vec<EnumRef>,
    seen: HashSet<String>,
    bare: Vec<String>,
}

impl EnumUses<'_> {
    fn note(&mut self, r: &EnumRef) {
        for arg in &r.args {
            self.ty(arg);
        }
        if !self.generic.contains_key(&r.name) {
            return;
        }
        if r.args.is_empty() {
            if !self.bare.contains(&r.name) {
                self.bare.push(r.name.clone());
            }
            return;
        }
        if self.seen.insert(r.symbol()) {
            self.wanted.push(r.clone());
        }
    }

    fn ty(&mut self, ty: &AirType) {
        match ty {
            AirType::Enum(r) => self.note(r),
            AirType::Ptr(inner)
            | AirType::Array(inner, _)
            | AirType::Slice(inner)
            | AirType::Vec(inner) => self.ty(inner),
            AirType::FnPtr { params, ret, .. } => {
                for param in params {
                    self.ty(param);
                }
                self.ty(ret);
            }
            _ => {}
        }
    }

    fn konst(&mut self, value: &AirConst) {
        match value {
            AirConst::Enum {
                enum_ref, payload, ..
            } => {
                self.note(enum_ref);
                for item in payload {
                    self.konst(item);
                }
            }
            AirConst::Struct { fields, .. } => {
                for (_, item) in fields {
                    self.konst(item);
                }
            }
            AirConst::Array(items) => {
                for item in items {
                    self.konst(item);
                }
            }
            AirConst::ZeroInit(ty) | AirConst::Undef(ty) => self.ty(ty),
            _ => {}
        }
    }

    fn operand(&mut self, operand: &Operand) {
        if let Operand::Const(value) = operand {
            self.konst(value);
        }
    }

    fn rvalue(&mut self, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::EnumInit {
                enum_ref, payload, ..
            } => {
                self.note(enum_ref);
                for op in payload {
                    self.operand(op);
                }
            }
            Rvalue::EnumTag { enum_ref, operand }
            | Rvalue::EnumPayload {
                enum_ref, operand, ..
            } => {
                self.note(enum_ref);
                self.operand(operand);
            }
            Rvalue::Cast { operand, from, to } => {
                self.operand(operand);
                self.ty(from);
                self.ty(to);
            }
            Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) | Rvalue::Len(op) => {
                self.operand(op)
            }
            Rvalue::BinaryOp(_, a, b) | Rvalue::Index { base: a, index: b } => {
                self.operand(a);
                self.operand(b);
            }
            Rvalue::Call { args, .. } => {
                for op in args {
                    self.operand(op);
                }
            }
            Rvalue::StructInit { fields, .. } => {
                for (_, op) in fields {
                    self.operand(op);
                }
            }
            Rvalue::FieldAccess { base, .. } => self.operand(base),
            Rvalue::ClosureCreate { env, .. } => self.operand(env),
            Rvalue::SliceFromParts { ptr, len } => {
                self.operand(ptr);
                self.operand(len);
            }
            Rvalue::AddressOf(_) => {}
        }
    }

    fn stmt(&mut self, stmt: &AirStmt) {
        match &stmt.kind {
            AirStmtKind::Assign { rvalue, .. } => self.rvalue(rvalue),
            AirStmtKind::GcAlloc { ty, .. }
            | AirStmtKind::Alloc { ty, .. }
            | AirStmtKind::RcAlloc { ty, .. } => self.ty(ty),
            AirStmtKind::CallVoid { args, .. } => {
                for op in args {
                    self.operand(op);
                }
            }
            _ => {}
        }
    }

    fn terminator(&mut self, term: &AirTerminator) {
        match term {
            AirTerminator::Return(Some(op)) => self.operand(op),
            AirTerminator::Branch { cond, .. } => self.operand(cond),
            AirTerminator::Switch { discr, .. } => self.operand(discr),
            AirTerminator::Invoke { args, .. } => {
                for op in args {
                    self.operand(op);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn substitute_type_params(
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
        AirType::Ptr(inner) => AirType::Ptr(Box::new(substitute_type_params(
            inner,
            type_params,
            type_args,
        ))),
        AirType::Array(inner, n) => AirType::Array(
            Box::new(substitute_type_params(inner, type_params, type_args)),
            *n,
        ),
        AirType::Slice(inner) => AirType::Slice(Box::new(substitute_type_params(
            inner,
            type_params,
            type_args,
        ))),
        AirType::Vec(inner) => AirType::Vec(Box::new(substitute_type_params(
            inner,
            type_params,
            type_args,
        ))),
        AirType::FnPtr { params, ret, conv } => AirType::FnPtr {
            params: params
                .iter()
                .map(|p| substitute_type_params(p, type_params, type_args))
                .collect(),
            ret: Box::new(substitute_type_params(ret, type_params, type_args)),
            conv: *conv,
        },
        AirType::Enum(r) => AirType::Enum(EnumRef {
            name: r.name.clone(),
            args: r
                .args
                .iter()
                .map(|a| substitute_type_params(a, type_params, type_args))
                .collect(),
        }),
        other => other.clone(),
    }
}

pub(super) struct MonoContext {
    pub(super) generic_functions: HashMap<String, usize>,
    pub(super) requests: Vec<MonoRequest>,
    pub(super) instantiated: HashMap<(String, Vec<String>), String>,
    pub(super) next_function_id: u32,
    pub(super) errors: Vec<MonoError>,
}

pub(super) struct ParamConflict {
    pub(super) param: u32,
    pub(super) first: AirType,
    pub(super) second: AirType,
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
            errors: Vec::new(),
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

        let mut conflicts: Vec<ParamConflict> = Vec::new();
        let inferred = self.infer_type_args(generic_func, args, caller, &mut conflicts);
        let name = name.clone();
        for conflict in conflicts {
            self.errors.push(MonoError::mono(
                Fault::Compiler,
                format!(
                    "`{}` binds type parameter {} to both `{}` and `{}` at a single call \
                     site, so no one instance answers it",
                    name,
                    conflict.param,
                    substitute::type_to_string(&conflict.first),
                    substitute::type_to_string(&conflict.second)
                ),
            ));
        }
        if let Some(type_args) = inferred {
            let key = (name.clone(), self.type_args_key(&type_args));
            if !self.instantiated.contains_key(&key) {
                self.requests.push(MonoRequest {
                    function_name: name,
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
        conflicts: &mut Vec<ParamConflict>,
    ) -> Option<Vec<AirType>> {
        let mut resolved: HashMap<u32, AirType> = HashMap::new();

        for (param, arg) in generic_func.params.iter().zip(args.iter()) {
            let arg_ty = operand_type_from(arg, &caller.params, &caller.locals);
            self.unify_param(
                &param.ty,
                &arg_ty,
                &generic_func.type_params,
                &mut resolved,
                conflicts,
            );
        }
        if !conflicts.is_empty() {
            return None;
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
        fn_type_params: &[TypeParamId],
        resolved: &mut HashMap<u32, AirType>,
        conflicts: &mut Vec<ParamConflict>,
    ) {
        match param_ty {
            AirType::Param(id) => match resolved.get(&id.0) {
                Some(bound) if bound != arg_ty => conflicts.push(ParamConflict {
                    param: id.0,
                    first: bound.clone(),
                    second: arg_ty.clone(),
                }),
                Some(_) => {}
                None => {
                    resolved.insert(id.0, arg_ty.clone());
                }
            },
            AirType::Ptr(inner) => {
                if let AirType::Ptr(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved, conflicts);
                }
            }
            AirType::Array(inner, _) => {
                if let AirType::Array(arg_inner, _) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved, conflicts);
                }
            }
            AirType::Slice(inner) => {
                if let AirType::Slice(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved, conflicts);
                }
            }
            AirType::Vec(inner) => {
                if let AirType::Vec(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved, conflicts);
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
                        self.unify_param(p, a, fn_type_params, resolved, conflicts);
                    }
                    self.unify_param(ret, arg_ret, fn_type_params, resolved, conflicts);
                }
            }
            AirType::Enum(param_ref) => {
                if let AirType::Enum(arg_ref) = arg_ty
                    && param_ref.name == arg_ref.name
                    && param_ref.args.len() == arg_ref.args.len()
                {
                    for (p, a) in param_ref.args.iter().zip(arg_ref.args.iter()) {
                        self.unify_param(p, a, fn_type_params, resolved, conflicts);
                    }
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
            .join("$");
        format!("__mono_{}_{}", name, type_str)
    }

    fn uninstantiated_call_sites(&self, program: &AirProgram) -> Vec<MonoError> {
        let mut names: Vec<String> = Vec::new();
        let note = |callee: &Callee, names: &mut Vec<String>| {
            if let Callee::Named(n) = callee
                && self.generic_functions.contains_key(n)
                && !names.contains(n)
            {
                names.push(n.clone());
            }
        };
        for func in &program.functions {
            for block in &func.blocks {
                for stmt in &block.stmts {
                    match &stmt.kind {
                        AirStmtKind::Assign {
                            rvalue: Rvalue::Call { func: callee, .. },
                            ..
                        }
                        | AirStmtKind::CallVoid { func: callee, .. } => note(callee, &mut names),
                        _ => {}
                    }
                }
                if let AirTerminator::Invoke { func: callee, .. } = &block.terminator {
                    note(callee, &mut names);
                }
            }
        }
        names
            .into_iter()
            .map(|n| {
                MonoError::mono(
                    Fault::Unsupported,
                    format!(
                        "`{n}` is generic and the call site does not determine its type \
                         arguments, so no instance was emitted; a type parameter that appears \
                         only inside a generic enum or struct type is not inferred yet"
                    ),
                )
            })
            .collect()
    }

    fn instantiate(&mut self, program: &mut AirProgram) -> Vec<MonoError> {
        let mut new_functions = Vec::new();
        let mut mono_instances = Vec::new();
        let mut errors = Vec::new();

        for request in &self.requests {
            let key = (
                request.function_name.clone(),
                self.type_args_key(&request.type_args),
            );

            if self.instantiated.contains_key(&key) {
                continue;
            }

            for ty in &request.type_args {
                let Some((kind, detail)) =
                    crate::passes::vec_surface::type_args_reject(program, ty)
                else {
                    continue;
                };
                match kind {
                    SurfaceErrorKind::NoDefinition => errors.push(MonoError {
                        kind: MonoErrorKind::NoDefinition,
                        fault: Fault::Compiler,
                        message: format!(
                            "{} `{}` is instantiated with a type argument for which {detail}",
                            crate::passes::vec_surface::NO_DEFINITION_MARKER,
                            request.function_name
                        ),
                    }),
                    SurfaceErrorKind::VecSurface => errors.push(MonoError::vec_surface(format!(
                        "{} `{}` is instantiated with a type argument for which {detail}; \
                         a Vec inside a generic instantiation is not supported yet (the buffer \
                         would be shared without a retain, the transitive Vec retain/release is \
                         not implemented)",
                        crate::passes::vec_surface::MARKER,
                        request.function_name
                    ))),
                }
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
        errors
    }
}
