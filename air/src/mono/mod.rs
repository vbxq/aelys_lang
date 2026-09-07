mod rewrite;
pub(crate) mod substitute;

use crate::*;
use std::collections::{HashMap, HashSet};
use substitute::operand_type_from;

const MONO_ROUNDS: usize = 64;

pub fn monomorphize(mut program: AirProgram) -> Result<AirProgram, Vec<String>> {
    let mut ctx = MonoContext::new(&program);
    // an instance body can itself call a generic, so collection reruns until it adds nothing
    for _ in 0..MONO_ROUNDS {
        ctx.requests.clear();
        ctx.collect_mono_requests(&program);
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
fn monomorphize_enums(program: &mut AirProgram) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();
    // collect generic enum indices
    let generic_enums: HashMap<String, usize> = program
        .enums
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.type_params.is_empty())
        .map(|(i, e)| (e.name.clone(), i))
        .collect();

    if generic_enums.is_empty() {
        return errors;
    }

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

    // also collect mono requests from pre-mangled local/param/return types.
    for func in &program.functions {
        for local in &func.locals {
            collect_premangled_enum_requests_from_type(
                &local.ty,
                &generic_enums,
                &program.enums,
                &mut enum_mono_requests,
            );
        }
        for param in &func.params {
            collect_premangled_enum_requests_from_type(
                &param.ty,
                &generic_enums,
                &program.enums,
                &mut enum_mono_requests,
            );
        }
        collect_premangled_enum_requests_from_type(
            &func.ret_ty,
            &generic_enums,
            &program.enums,
            &mut enum_mono_requests,
        );
    }

    if enum_mono_requests.is_empty() {
        return errors;
    }

    // create monomorphized enum definitions
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

    // phase 2: build per-local mono resolution map for each function
    for func in &mut program.functions {
        let local_mono_map =
            resolve_local_enum_monos(func, &generic_enums, &program.enums, &mono_enum_names);

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
                    &mut errors,
                );
            }
        }
        // rewrite local types that reference generic enums
        for local in &mut func.locals {
            if let AirType::Enum(ref name) = local.ty {
                if generic_enums.contains_key(name) {
                    if let Some(mangled) = local_mono_map.get(&local.id) {
                        local.ty = AirType::Enum(mangled.clone());
                    } else {
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

    // remove generic enum definitions (they've been replaced by mono'd versions)
    program.enums.retain(|e| e.type_params.is_empty());

    errors
}

/// build a per-local mapping from `localid` to monomorphized enum name.
fn resolve_local_enum_monos(
    func: &AirFunction,
    generic_enums: &HashMap<String, usize>,
    enum_defs: &[AirEnumDef],
    mono_enum_names: &HashMap<(String, Vec<String>), String>,
) -> HashMap<LocalId, String> {
    let mut local_mono: HashMap<LocalId, String> = HashMap::new();

    // pass 0: if a local's or param's type was pre-mangled by the lowering pass
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

    for block in &func.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(local_id),
                rvalue,
            } = &stmt.kind
            {
                if let Rvalue::EnumInit {
                    enum_name, payload, ..
                } = rvalue
                {
                    if payload.is_empty() {
                        continue; // unit variant, skip for now
                    }
                    if let Some(&enum_idx) = generic_enums.get(enum_name.as_str()) {
                        let enum_def = &enum_defs[enum_idx];
                        if let Some(type_args) = infer_enum_type_args(enum_def, rvalue, func) {
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

    let ret_ty_mono = if let AirType::Enum(ref name) = func.ret_ty {
        if name.starts_with("__mono_") {
            // already monomorphized (e.g., from function monomorphization)
            Some(name.clone())
        } else if generic_enums.contains_key(name) {
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

    // return type is a known mono enum, that local should use the same mono name.
    if let Some(ref ret_mono) = ret_ty_mono {
        for block in &func.blocks {
            if let AirTerminator::Return(Some(Operand::Copy(local_id))) = &block.terminator {
                let local_ty = func
                    .locals
                    .iter()
                    .find(|l| l.id == *local_id)
                    .map(|l| &l.ty);
                if let Some(AirType::Enum(name)) = local_ty {
                    if generic_enums.contains_key(name) {
                        local_mono
                            .entry(*local_id)
                            .or_insert_with(|| ret_mono.clone());
                    }
                }
            }
        }
    }

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

    // pass 3: for remaining unresolved locals with generic enum types,
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
        AirType::Vec(inner) => AirType::Vec(Box::new(substitute_enum_type(
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
            // the enum name may contain pre-mangled param references (e.g.
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
                if let Some(type_args) = infer_enum_type_args(enum_def, rvalue, func) {
                    let key_strs: Vec<String> =
                        type_args.iter().map(substitute::type_to_string).collect();
                    let key = (enum_name.clone(), key_strs);
                    requests.entry(key).or_insert(type_args);
                } else if payload.is_empty() {
                }
            }
        }
        Rvalue::EnumTag { enum_name, .. } | Rvalue::EnumPayload { enum_name, .. } => {
            if generic_enums.contains_key(enum_name) {
            }
        }
        _ => {}
    }
}

fn collect_premangled_enum_requests_from_type(
    ty: &AirType,
    generic_enums: &HashMap<String, usize>,
    enum_defs: &[AirEnumDef],
    requests: &mut HashMap<(String, Vec<String>), Vec<AirType>>,
) {
    match ty {
        AirType::Enum(name) => {
            if !name.starts_with("__mono_") {
                return;
            }
            for (enum_name, &enum_idx) in generic_enums {
                let prefix = format!("__mono_{}_", enum_name);
                if let Some(type_suffix) = name.strip_prefix(&prefix) {
                    let enum_def = &enum_defs[enum_idx];
                    if let Some(type_args) = resolve_type_args_from_suffix(type_suffix, enum_def) {
                        let key_strs: Vec<String> =
                            type_args.iter().map(substitute::type_to_string).collect();
                        let key = (enum_name.clone(), key_strs);
                        let inserted = requests
                            .entry(key)
                            .or_insert_with(|| type_args.clone())
                            .clone();
                        for nested in &inserted {
                            collect_premangled_enum_requests_from_type(
                                nested,
                                generic_enums,
                                enum_defs,
                                requests,
                            );
                        }
                    }
                    break;
                }
            }
        }
        AirType::Ptr(inner) | AirType::Slice(inner) | AirType::Vec(inner) => {
            collect_premangled_enum_requests_from_type(inner, generic_enums, enum_defs, requests);
        }
        AirType::Array(inner, _) => {
            collect_premangled_enum_requests_from_type(inner, generic_enums, enum_defs, requests);
        }
        AirType::FnPtr { params, ret, .. } => {
            for param in params {
                collect_premangled_enum_requests_from_type(
                    param,
                    generic_enums,
                    enum_defs,
                    requests,
                );
            }
            collect_premangled_enum_requests_from_type(ret, generic_enums, enum_defs, requests);
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
        AirType::Vec(inner) => {
            if let AirType::Vec(arg_inner) = arg_ty {
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
    errors: &mut Vec<String>,
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
                                    resolved_from_payload = true;
                                }
                            }
                        }
                    }
                    if !resolved_from_payload {
                        // unit variant: use the target local's resolved mono name
                        if let Place::Local(target_id) = place {
                            if let Some(mangled) = local_mono_map.get(target_id) {
                                *enum_name = mangled.clone();
                            } else {
                                let mono_names: Vec<_> = mono_enum_names
                                    .iter()
                                    .filter(|((en, _), _)| en == enum_name.as_str())
                                    .map(|(_, mn)| mn.clone())
                                    .collect();
                                if mono_names.len() == 1 {
                                    *enum_name = mono_names[0].clone();
                                } else if mono_names.len() > 1 {
                                    errors.push(format!(
                                        "ambiguous unit variant {}::{} with {} \
                                         monomorphizations; cannot determine which to use \
                                         (type annotation info lost during sema)",
                                        enum_name,
                                        variant,
                                        mono_names.len()
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            Rvalue::EnumTag {
                enum_name, operand, ..
            } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    if let Some(mangled) = resolve_operand_mono(operand, local_mono_map) {
                        *enum_name = mangled;
                    } else {
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
                enum_name, operand, ..
            } => {
                if generic_enums.contains_key(enum_name.as_str()) {
                    if let Some(mangled) = resolve_operand_mono(operand, local_mono_map) {
                        *enum_name = mangled;
                    } else {
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

/// resolve the monomorphized enum name from an operand by looking up the local in
fn resolve_operand_mono(
    operand: &Operand,
    local_mono_map: &HashMap<LocalId, String>,
) -> Option<String> {
    match operand {
        Operand::Copy(id) | Operand::Move(id) => local_mono_map.get(id).cloned(),
        Operand::Const(_) => None,
    }
}

// a bare generic enum in a signature is mangled with the function type args, not the enum's own
fn decode_generic_enum_arg(base: &str, arg_ty: &AirType, arity: usize) -> Vec<AirType> {
    if arity == 0 || base.starts_with("__mono_") {
        return Vec::new();
    }
    let AirType::Enum(arg_name) = arg_ty else {
        return Vec::new();
    };
    let Some(suffix) = arg_name.strip_prefix(&format!("__mono_{base}_")) else {
        return Vec::new();
    };
    let segments: Vec<&str> = suffix.split('$').collect();
    resolve_type_args_from_segments(&segments, arity).unwrap_or_default()
}

fn resolve_type_args_from_suffix(type_suffix: &str, enum_def: &AirEnumDef) -> Option<Vec<AirType>> {
    let arity = enum_def.type_params.len();
    if arity == 0 {
        return Some(Vec::new());
    }
    let segments: Vec<&str> = type_suffix.split('$').collect();
    resolve_type_args_from_segments(&segments, arity)
}

fn resolve_type_args_from_segments(segments: &[&str], remaining: usize) -> Option<Vec<AirType>> {
    if remaining == 0 {
        return segments.is_empty().then_some(Vec::new());
    }
    if segments.len() < remaining {
        return None;
    }

    let max_take = segments.len() - remaining + 1;
    for take in (1..=max_take).rev() {
        let candidate = segments[..take].join("$");
        if let Some(ty) = resolve_single_type_arg(&candidate)
            && let Some(mut rest) =
                resolve_type_args_from_segments(&segments[take..], remaining - 1)
        {
            let mut type_args = vec![ty];
            type_args.append(&mut rest);
            return Some(type_args);
        }
    }

    None
}

fn resolve_type_list_from_segments(segments: &[&str]) -> Option<Vec<AirType>> {
    if segments.is_empty() {
        return Some(Vec::new());
    }

    for take in (1..=segments.len()).rev() {
        let candidate = segments[..take].join("$");
        if let Some(ty) = resolve_single_type_arg(&candidate)
            && let Some(mut rest) = resolve_type_list_from_segments(&segments[take..])
        {
            let mut items = vec![ty];
            items.append(&mut rest);
            return Some(items);
        }
    }

    None
}

fn resolve_single_type_arg(s: &str) -> Option<AirType> {
    let ty = match s {
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
        "opaque" => AirType::Opaque,
        "void" => AirType::Void,
        other => {
            if let Some(rest) = other.strip_prefix("ptr_") {
                AirType::Ptr(Box::new(resolve_single_type_arg(rest)?))
            } else if let Some(rest) = other.strip_prefix("slice_") {
                AirType::Slice(Box::new(resolve_single_type_arg(rest)?))
            } else if let Some(rest) = other.strip_prefix("vec_") {
                AirType::Vec(Box::new(resolve_single_type_arg(rest)?))
            } else if let Some(rest) = other.strip_prefix("array_") {
                let split = rest.rfind('_')?;
                let inner = &rest[..split];
                let n = rest[split + 1..].parse().ok()?;
                AirType::Array(Box::new(resolve_single_type_arg(inner)?), n)
            } else if let Some((rest, conv)) = other
                .strip_prefix("fnptrRust$")
                .map(|rest| (rest, CallingConv::Rust))
                .or_else(|| {
                    other
                        .strip_prefix("fnptrC$")
                        .map(|rest| (rest, CallingConv::C))
                })
                .or_else(|| {
                    other
                        .strip_prefix("fnptr$")
                        .map(|rest| (rest, CallingConv::Aelys))
                })
            {
                let (params, ret) = if let Some(ret) = rest.strip_prefix("$R") {
                    (Vec::new(), resolve_single_type_arg(ret)?)
                } else {
                    let segments: Vec<&str> = rest.split('$').collect();
                    let mut parsed: Option<(Vec<AirType>, AirType)> = None;
                    for ret_start in 1..segments.len() {
                        let ret_head = match segments[ret_start].strip_prefix('R') {
                            Some(head) => head,
                            None => continue,
                        };
                        let mut ret_segments = Vec::with_capacity(segments.len() - ret_start);
                        ret_segments.push(ret_head);
                        ret_segments.extend_from_slice(&segments[ret_start + 1..]);
                        let Some(params) = resolve_type_list_from_segments(&segments[..ret_start])
                        else {
                            continue;
                        };
                        let Some(ret) = resolve_single_type_arg(&ret_segments.join("$")) else {
                            continue;
                        };
                        parsed = Some((params, ret));
                        break;
                    }
                    parsed?
                };
                AirType::FnPtr {
                    params,
                    ret: Box::new(ret),
                    conv,
                }
            } else if let Some(rest) = other.strip_prefix("enum_") {
                AirType::Enum(rest.to_string())
            } else if let Some(rest) = other.strip_prefix("param_") {
                AirType::Param(TypeParamId(rest.parse().ok()?))
            } else {
                if other.contains('$') && !other.starts_with("__mono_") {
                    return None;
                }
                AirType::Struct(other.to_string())
            }
        }
    };
    Some(ty)
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
            self.unify_param(&param.ty, &arg_ty, &generic_func.type_params, &mut resolved);
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
    ) {
        match param_ty {
            AirType::Param(id) => {
                resolved.entry(id.0).or_insert_with(|| arg_ty.clone());
            }
            AirType::Ptr(inner) => {
                if let AirType::Ptr(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved);
                }
            }
            AirType::Array(inner, _) => {
                if let AirType::Array(arg_inner, _) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved);
                }
            }
            AirType::Slice(inner) => {
                if let AirType::Slice(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved);
                }
            }
            AirType::Vec(inner) => {
                if let AirType::Vec(arg_inner) = arg_ty {
                    self.unify_param(inner, arg_inner, fn_type_params, resolved);
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
                        self.unify_param(p, a, fn_type_params, resolved);
                    }
                    self.unify_param(ret, arg_ret, fn_type_params, resolved);
                }
            }
            AirType::Enum(base) => {
                for (slot, ty) in decode_generic_enum_arg(base, arg_ty, fn_type_params.len())
                    .into_iter()
                    .enumerate()
                {
                    if let Some(tp) = fn_type_params.get(slot) {
                        resolved.entry(tp.0).or_insert(ty);
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

    fn uninstantiated_call_sites(&self, program: &AirProgram) -> Vec<String> {
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
                format!(
                    "`{n}` is generic and the call site does not determine its type arguments, so \
                     no instance was emitted; a type parameter that appears only inside a generic \
                     enum or struct type is not inferred yet"
                )
            })
            .collect()
    }

    fn instantiate(&mut self, program: &mut AirProgram) -> Vec<String> {
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
                if let Some(detail) = crate::passes::vec_surface::type_args_reject(program, ty) {
                    errors.push(format!(
                        "{} `{}` is instantiated with a type argument for which {detail}; \
                         a Vec inside a generic instantiation is not supported yet (the buffer \
                         would be shared without a retain, the transitive Vec retain/release is \
                         not implemented)",
                        crate::passes::vec_surface::MARKER,
                        request.function_name
                    ));
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
