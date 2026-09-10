use crate::*;

pub(crate) fn type_to_string(ty: &AirType) -> String {
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
        AirType::Enum(r) => format!("enum_{}", r.symbol()),
        AirType::Array(inner, size) => format!("array_{}_{}", type_to_string(inner), size),
        AirType::Slice(inner) => format!("slice_{}", type_to_string(inner)),
        AirType::Vec(inner) => format!("vec_{}", type_to_string(inner)),
        AirType::FnPtr { params, ret, conv } => {
            let params_str = params
                .iter()
                .map(type_to_string)
                .collect::<Vec<_>>()
                .join("$");
            // calling convention is part of the fnptr type; omitting it aliases
            let prefix = match conv {
                CallingConv::Aelys => "fnptr",
                CallingConv::C => "fnptrC",
                CallingConv::Rust => "fnptrRust",
            };
            format!("{prefix}${}$R{}", params_str, type_to_string(ret))
        }
        AirType::Param(id) => format!("param_{}", id.0),
        AirType::Opaque => "opaque".to_string(),
        AirType::Void => "void".to_string(),
    }
}

pub(super) fn substitute_types_in_function(
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
        AirType::Vec(inner) => substitute_type(inner, type_params, type_args),
        AirType::FnPtr { params, ret, .. } => {
            for p in params {
                substitute_type(p, type_params, type_args);
            }
            substitute_type(ret, type_params, type_args);
        }
        AirType::Enum(r) => {
            for arg in &mut r.args {
                substitute_type(arg, type_params, type_args);
            }
        }
        _ => {}
    }
}

fn substitute_stmt(stmt: &mut AirStmt, type_params: &[TypeParamId], type_args: &[AirType]) {
    match &mut stmt.kind {
        AirStmtKind::Assign { rvalue, .. } => {
            substitute_rvalue(rvalue, type_params, type_args);
        }
        AirStmtKind::GcAlloc { ty, .. }
        | AirStmtKind::Alloc { ty, .. }
        | AirStmtKind::RcAlloc { ty, .. } => {
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
        Rvalue::EnumInit { enum_ref, .. }
        | Rvalue::EnumTag { enum_ref, .. }
        | Rvalue::EnumPayload { enum_ref, .. } => {
            for arg in &mut enum_ref.args {
                substitute_type(arg, type_params, type_args);
            }
        }
        Rvalue::ClosureCreate { .. } => {}
        Rvalue::SliceFromParts { .. } => {}
        _ => {}
    }
}

fn substitute_terminator(
    _term: &mut AirTerminator,
    _type_params: &[TypeParamId],
    _type_args: &[AirType],
) {
}

pub(super) fn operand_type_from(
    operand: &Operand,
    params: &[AirParam],
    locals: &[AirLocal],
) -> AirType {
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
            AirConst::Null => AirType::Ptr(Box::new(AirType::Void)),
            AirConst::FnRef(_) => AirType::Ptr(Box::new(AirType::Void)),
            AirConst::Enum { enum_ref, .. } => AirType::Enum(enum_ref.clone()),
            AirConst::ZeroInit(ty) | AirConst::Undef(ty) => ty.clone(),
            AirConst::Array(_) => AirType::Opaque, // type not recoverable without element info
            AirConst::Struct { name, .. } => AirType::Struct(name.clone()),
        },
        Operand::Copy(id) | Operand::Move(id) => params
            .iter()
            .find(|p| p.id == *id)
            .map(|p| p.ty.clone())
            .or_else(|| locals.iter().find(|l| l.id == *id).map(|l| l.ty.clone()))
            .unwrap_or_else(|| {
                panic!(
                    "invariant: mono operand_type_from: local %{} not found in params or locals",
                    id.0
                )
            }),
    }
}
