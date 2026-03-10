mod rewrite;
pub(crate) mod substitute;

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
///
/// The algorithm has three phases:
///
/// 1. **Collection**: Scan all `EnumInit` sites with non-empty payload to infer
///    `(enum_name, type_args)` pairs. Unit variants cannot contribute type args here.
///
/// 2. **Local resolution**: Build a per-local mapping `LocalId -> mangled_name` by:
///    - Looking at non-unit `EnumInit` assignments to each local.
///    - Propagating from the function return type (for return statements).
///    - Propagating from already-resolved locals (for `Rvalue::Use(Copy(id))`).
///    - Falling back to a unique match when only one monomorphization exists.
///
/// 3. **Rewriting**: Use the local mapping to rewrite unit variant `EnumInit`,
///    `EnumTag`, `EnumPayload`, and local types.
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

    // Phase 1: Collect all (enum_name, type_args) pairs from EnumInit sites
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

    // Also collect mono requests from pre-mangled local/param/return types.
    // When sema preserves type args (e.g., Option<i64>), the lowering pass pre-computes
    // the mangled name. We need to ensure the corresponding enum definition exists.
    for func in &program.functions {
        let mut collect_premangled = |name: &str| {
            if !name.starts_with("__mono_") {
                return;
            }
            // Parse "__mono_{enum_name}_{type_suffix}" to find the original enum
            for (enum_name, &enum_idx) in &generic_enums {
                let prefix = format!("__mono_{}_", enum_name);
                if let Some(type_suffix) = name.strip_prefix(&prefix) {
                    // Check if we already have a request for this
                    let key_strs: Vec<String> =
                        type_suffix.split('$').map(|s| s.to_string()).collect();
                    let key = (enum_name.clone(), key_strs.clone());
                    if !enum_mono_requests.contains_key(&key) {
                        // Try to resolve type_args from the type suffix strings
                        let enum_def = &program.enums[enum_idx];
                        if let Some(type_args) =
                            resolve_type_args_from_suffix(&key_strs, enum_def)
                        {
                            enum_mono_requests.insert(key, type_args);
                        }
                    }
                    break;
                }
            }
        };

        for local in &func.locals {
            if let AirType::Enum(ref name) = local.ty {
                collect_premangled(name);
            }
        }
        for param in &func.params {
            if let AirType::Enum(ref name) = param.ty {
                collect_premangled(name);
            }
        }
        if let AirType::Enum(ref name) = func.ret_ty {
            collect_premangled(name);
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
            .join("$");
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

    // Phase 2: Build per-local mono resolution map for each function
    // Phase 3: Rewrite enum_name references in all functions
    for func in &mut program.functions {
        let local_mono_map = resolve_local_enum_monos(
            func,
            &generic_enums,
            &program.enums,
            &mono_enum_names,
        );

        for block in &mut func.blocks {
            for stmt in &mut block.stmts {
                rewrite_enum_refs_in_stmt(
                    stmt,
                    &generic_enums,
                    &program.enums,
                    &mono_enum_names,
                    &func.params.clone(),
                    &func.locals.clone(),
                    &local_mono_map,
                );
            }
        }
        // Rewrite local types that reference generic enums
        for local in &mut func.locals {
            if let AirType::Enum(ref name) = local.ty {
                if generic_enums.contains_key(name) {
                    // First try the resolved map from assignments
                    if let Some(mangled) = local_mono_map.get(&local.id) {
                        local.ty = AirType::Enum(mangled.clone());
                    } else {
                        // Fall back to unique match
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
        // Also rewrite param types
        for param in &mut func.params {
            if let AirType::Enum(ref name) = param.ty {
                if generic_enums.contains_key(name) {
                    let mono_names: Vec<_> = mono_enum_names
                        .iter()
                        .filter(|((en, _), _)| en == name)
                        .map(|(_, mn)| mn.clone())
                        .collect();
                    if mono_names.len() == 1 {
                        param.ty = AirType::Enum(mono_names[0].clone());
                    }
                }
            }
        }
        // Rewrite function return type
        if let AirType::Enum(ref name) = func.ret_ty {
            if generic_enums.contains_key(name) {
                let mono_names: Vec<_> = mono_enum_names
                    .iter()
                    .filter(|((en, _), _)| en == name)
                    .map(|(_, mn)| mn.clone())
                    .collect();
                if mono_names.len() == 1 {
                    func.ret_ty = AirType::Enum(mono_names[0].clone());
                }
            }
        }
    }

    // Remove generic enum definitions (they've been replaced by mono'd versions)
    program.enums.retain(|e| e.type_params.is_empty());
}

/// Build a per-local mapping from `LocalId` to monomorphized enum name.
///
/// For each local that holds a generic enum type, try to determine which specific
/// monomorphization it should use by examining:
/// 1. Non-unit `EnumInit` assignments to the local (payload types give us type args)
/// 2. `Rvalue::Use(Copy(other_local))` assignments (propagate from already-resolved locals)
/// 3. Function return type context (for locals used in return statements)
/// 4. Unique-match fallback (when only one monomorphization exists)
fn resolve_local_enum_monos(
    func: &AirFunction,
    generic_enums: &HashMap<String, usize>,
    enum_defs: &[AirEnumDef],
    mono_enum_names: &HashMap<(String, Vec<String>), String>,
) -> HashMap<LocalId, String> {
    let mut local_mono: HashMap<LocalId, String> = HashMap::new();

    // Pass 0: If a local's or param's type was pre-mangled by the lowering pass
    // (sema had concrete type args in the annotation), use that directly.
    for local in &func.locals {
        if let AirType::Enum(ref name) = local.ty {
            if name.starts_with("__mono_") {
                local_mono.insert(local.id, name.clone());
            }
        }
    }
    for param in &func.params {
        if let AirType::Enum(ref name) = param.ty {
            if name.starts_with("__mono_") {
                local_mono.insert(param.id, name.clone());
            }
        }
    }

    // Pass 1: Resolve locals that have non-unit EnumInit assignments
    for block in &func.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(local_id),
                rvalue,
            } = &stmt.kind
            {
                if let Rvalue::EnumInit {
                    enum_name,
                    payload,
                    ..
                } = rvalue
                {
                    if payload.is_empty() {
                        continue; // unit variant, skip for now
                    }
                    if let Some(&enum_idx) = generic_enums.get(enum_name.as_str()) {
                        let enum_def = &enum_defs[enum_idx];
                        if let Some(type_args) =
                            infer_enum_type_args(enum_def, rvalue, func)
                        {
                            let key_strs: Vec<String> =
                                type_args.iter().map(substitute::type_to_string).collect();
                            let key = (enum_name.clone(), key_strs);
                            if let Some(mangled) = mono_enum_names.get(&key) {
                                local_mono.insert(*local_id, mangled.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    // Pass 2: Propagate through Use(Copy(other)) assignments and return type context
    // Also check function return type for locals that appear in return terminators
    let ret_ty_mono = if let AirType::Enum(ref name) = func.ret_ty {
        if name.starts_with("__mono_") {
            // Already monomorphized (e.g., from function monomorphization)
            Some(name.clone())
        } else if generic_enums.contains_key(name) {
            // Try unique match for the return type
            let mono_names: Vec<_> = mono_enum_names
                .iter()
                .filter(|((en, _), _)| en == name)
                .map(|(_, mn)| mn.clone())
                .collect();
            if mono_names.len() == 1 {
                Some(mono_names[0].clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Propagate from return terminators: if a local is returned and the function
    // return type is a known mono enum, that local should use the same mono name.
    if let Some(ref ret_mono) = ret_ty_mono {
        for block in &func.blocks {
            if let AirTerminator::Return(Some(Operand::Copy(local_id))) = &block.terminator {
                let local_ty = func.locals.iter().find(|l| l.id == *local_id).map(|l| &l.ty);
                if let Some(AirType::Enum(name)) = local_ty {
                    if generic_enums.contains_key(name) {
                        local_mono.entry(*local_id).or_insert_with(|| ret_mono.clone());
                    }
                }
            }
        }
    }

    // Propagate from Use(Copy(source)) assignments
    let mut changed = true;
    while changed {
        changed = false;
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign {
                    place: Place::Local(target_id),
                    rvalue: Rvalue::Use(Operand::Copy(source_id)),
                } = &stmt.kind
                {
                    if !local_mono.contains_key(target_id) {
                        if let Some(mangled) = local_mono.get(source_id).cloned() {
                            local_mono.insert(*target_id, mangled);
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Pass 3: For remaining unresolved locals with generic enum types,
    // try unique-match fallback
    for local in &func.locals {
        if local_mono.contains_key(&local.id) {
            continue;
        }
        if let AirType::Enum(ref name) = local.ty {
            if generic_enums.contains_key(name) {
                let mono_names: Vec<_> = mono_enum_names
                    .iter()
                    .filter(|((en, _), _)| en == name)
                    .map(|(_, mn)| mn.clone())
                    .collect();
                if mono_names.len() == 1 {
                    local_mono.insert(local.id, mono_names[0].clone());
                }
            }
        }
    }

    local_mono
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
        AirType::Enum(name) => {
            // The enum name may contain pre-mangled param references (e.g.
            // "__mono_Option_param_0") when a generic enum definition has a
            // variant whose payload is another generic enum parameterized by a
            // type param. Replace each "param_N" segment with the concrete
            // type arg so the name resolves to the correct monomorphized def.
            let mut new_name = name.clone();
            for (i, param) in type_params.iter().enumerate() {
                if let Some(replacement) = type_args.get(i) {
                    let param_str = substitute::type_to_string(&AirType::Param(*param));
                    let replacement_str = substitute::type_to_string(replacement);
                    if param_str != replacement_str {
                        new_name = new_name.replace(&param_str, &replacement_str);
                    }
                }
            }
            AirType::Enum(new_name)
        }
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
    local_mono_map: &HashMap<LocalId, String>,
) {
    if let AirStmtKind::Assign { place, rvalue } = &mut stmt.kind {
        match rvalue {
            Rvalue::EnumInit {
                enum_name,
                variant,
                payload,
                ..
            } => {
                if let Some(&enum_idx) = generic_enums.get(enum_name.as_str()) {
                    let enum_def = &enum_defs[enum_idx];
                    let mut resolved_from_payload = false;
                    if !payload.is_empty() {
                        // Non-unit variant: infer type args from payload
                        let variant_def = enum_def.variants.iter().find(|v| v.name == *variant);
                        if let Some(vd) = variant_def {
                            let mut resolved: HashMap<u32, AirType> = HashMap::new();
                            for (param_ty, operand) in vd.payload.iter().zip(payload.iter()) {
                                let arg_ty =
                                    operand_type_from(operand, func_params, func_locals);
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
                                    resolved_from_payload = true;
                                }
                            }
                        }
                    }
                    if !resolved_from_payload {
                        // Unit variant: use the target local's resolved mono name
                        if let Place::Local(target_id) = place {
                            if let Some(mangled) = local_mono_map.get(target_id) {
                                *enum_name = mangled.clone();
                            } else {
                                // Fallback: unique match
                                let mono_names: Vec<_> = mono_enum_names
                                    .iter()
                                    .filter(|((en, _), _)| en == enum_name.as_str())
                                    .map(|(_, mn)| mn.clone())
                                    .collect();
                                if mono_names.len() == 1 {
                                    *enum_name = mono_names[0].clone();
                                } else if mono_names.len() > 1 {
                                    eprintln!(
                                        "[AIR] warning: ambiguous unit variant {}::{} with {} \
                                         monomorphizations; cannot determine which to use \
                                         (type annotation info lost during sema). \
                                         Using first available.",
                                        enum_name, variant, mono_names.len()
                                    );
                                    *enum_name = mono_names[0].clone();
                                }
                            }
                        }
                    }
                }
            }
            Rvalue::EnumTag {
                enum_name,
                operand,
                ..
            } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    // Resolve from the operand's local
                    if let Some(mangled) = resolve_operand_mono(operand, local_mono_map) {
                        *enum_name = mangled;
                    } else {
                        // Fallback: unique match
                        let mono_names: Vec<_> = mono_enum_names
                            .iter()
                            .filter(|((en, _), _)| en == enum_name.as_str())
                            .map(|(_, mn)| mn.clone())
                            .collect();
                        if mono_names.len() == 1 {
                            *enum_name = mono_names[0].clone();
                        }
                    }
                }
            }
            Rvalue::EnumPayload {
                enum_name,
                operand,
                ..
            } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    // Resolve from the operand's local
                    if let Some(mangled) = resolve_operand_mono(operand, local_mono_map) {
                        *enum_name = mangled;
                    } else {
                        // Fallback: unique match
                        let mono_names: Vec<_> = mono_enum_names
                            .iter()
                            .filter(|((en, _), _)| en == enum_name.as_str())
                            .map(|(_, mn)| mn.clone())
                            .collect();
                        if mono_names.len() == 1 {
                            *enum_name = mono_names[0].clone();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Resolve the monomorphized enum name from an operand by looking up the local in
/// the local_mono_map.
fn resolve_operand_mono(
    operand: &Operand,
    local_mono_map: &HashMap<LocalId, String>,
) -> Option<String> {
    match operand {
        Operand::Copy(id) | Operand::Move(id) => local_mono_map.get(id).cloned(),
        Operand::Const(_) => None,
    }
}

/// Resolve AirType values from suffix strings produced by `type_to_string`.
///
/// Each string in `key_strs` is a single type arg (split by `$` separator).
/// Handles primitives, enum types (prefixed with "enum_"), and struct types.
fn resolve_type_args_from_suffix(
    key_strs: &[String],
    _enum_def: &AirEnumDef,
) -> Option<Vec<AirType>> {
    let mut type_args = Vec::new();
    for s in key_strs {
        let ty = match s.as_str() {
            "i8" => AirType::I8,
            "i16" => AirType::I16,
            "i32" => AirType::I32,
            "i64" => AirType::I64,
            "u8" => AirType::U8,
            "u16" => AirType::U16,
            "u32" => AirType::U32,
            "u64" => AirType::U64,
            "f32" => AirType::F32,
            "f64" => AirType::F64,
            "bool" => AirType::Bool,
            "str" => AirType::Str,
            other => {
                if let Some(enum_name) = other.strip_prefix("enum_") {
                    AirType::Enum(enum_name.to_string())
                } else {
                    AirType::Struct(other.to_string())
                }
            }
        };
        type_args.push(ty);
    }
    Some(type_args)
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
            .join("$");
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
