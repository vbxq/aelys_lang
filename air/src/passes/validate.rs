//! - eo `airtype::opaque` anywhere, because this means an unresolved dynamic type survived past monomorphization

use crate::{
    AirBlock, AirFunction, AirProgram, AirStmtKind, AirTerminator, AirType, BlockId, Callee,
    LocalId, Operand, Place, Rvalue,
};
use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone)]
pub struct AirValidationError {
    pub function_name: String,
    pub detail: AirValidationDetail,
}

#[derive(Debug, Clone)]
pub enum AirValidationDetail {
    VoidLocal {
        local_id: u32,
        local_name: Option<String>,
    },
    UndeclaredLocal { local_id: u32, context: String },
    UndeclaredBlock { block_id: u32, context: String },
    /// a local or param has type opaque (unresolved dynamic that survived monomorphization).
    OpaqueType {
        local_id: u32,
        local_name: Option<String>,
    },
    OpaqueStructField {
        struct_name: String,
        field_name: String,
    },
    UnknownEnumType {
        local_id: u32,
        local_name: Option<String>,
        enum_name: String,
    },
    UnknownStructFieldEnum {
        struct_name: String,
        field_name: String,
        enum_name: String,
    },
    UnknownEnumReference { enum_name: String, context: String },
    UnknownGlobalReference {
        global_name: String,
        context: String,
    },
    EmptyBody,
    UnwrittenTerminatorOperand {
        local_id: u32,
        local_name: Option<String>,
        context: String,
    },
    PtrnessMismatch {
        context: String,
        rvalue: &'static str,
        place_ty: String,
    },
}

impl fmt::Display for AirValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AIR validation error in `{}`: ", self.function_name)?;
        match &self.detail {
            AirValidationDetail::VoidLocal {
                local_id,
                local_name,
            } => {
                write!(f, "local %{local_id}")?;
                if let Some(name) = local_name {
                    write!(f, " (`{name}`)")?;
                }
                write!(
                    f,
                    " has type Void, which is invalid outside return position"
                )
            }
            AirValidationDetail::UndeclaredLocal {
                local_id, context, ..
            } => {
                write!(f, "local %{local_id} is used but not declared ({context})")
            }
            AirValidationDetail::UndeclaredBlock {
                block_id, context, ..
            } => {
                write!(
                    f,
                    "block bb{block_id} is referenced but not declared ({context})"
                )
            }
            AirValidationDetail::OpaqueType {
                local_id,
                local_name,
            } => {
                write!(f, "local %{local_id}")?;
                if let Some(name) = local_name {
                    write!(f, " (`{name}`)")?;
                }
                write!(
                    f,
                    " has unresolved Dynamic type (Opaque), inference or monomorphization did not resolve this type"
                )
            }
            AirValidationDetail::OpaqueStructField {
                struct_name,
                field_name,
            } => {
                write!(
                    f,
                    "struct `{struct_name}` field `{field_name}` has unresolved Dynamic type (Opaque)"
                )
            }
            AirValidationDetail::UnknownEnumType {
                local_id,
                local_name,
                enum_name,
            } => {
                write!(f, "local %{local_id}")?;
                if let Some(name) = local_name {
                    write!(f, " (`{name}`)")?;
                }
                write!(
                    f,
                    " references unknown enum `{enum_name}` after monomorphization"
                )
            }
            AirValidationDetail::UnknownStructFieldEnum {
                struct_name,
                field_name,
                enum_name,
            } => {
                write!(
                    f,
                    "struct `{struct_name}` field `{field_name}` references unknown enum `{enum_name}` after monomorphization"
                )
            }
            AirValidationDetail::UnknownEnumReference { enum_name, context } => {
                write!(
                    f,
                    "enum operation references unknown enum `{enum_name}` after monomorphization ({context})"
                )
            }
            AirValidationDetail::UnknownGlobalReference {
                global_name,
                context,
            } => {
                write!(
                    f,
                    "place references unknown global `{global_name}` ({context})"
                )
            }
            AirValidationDetail::EmptyBody => {
                write!(f, "non-extern function has no basic blocks")
            }
            AirValidationDetail::UnwrittenTerminatorOperand {
                local_id,
                local_name,
                context,
            } => {
                write!(f, "local %{local_id}")?;
                if let Some(name) = local_name {
                    write!(f, " (`{name}`)")?;
                }
                write!(
                    f,
                    " is read by a terminator but is never assigned in the function ({context})"
                )
            }
            AirValidationDetail::PtrnessMismatch {
                context,
                rvalue,
                place_ty,
            } => {
                write!(
                    f,
                    "`{rvalue}` assigns into a place of type {place_ty} ({context})"
                )
            }
        }
    }
}

fn place_type(function: &AirFunction, program: &AirProgram, place: &Place) -> Option<AirType> {
    match place {
        Place::Local(id) => function
            .params
            .iter()
            .find(|p| p.id == *id)
            .map(|p| p.ty.clone())
            .or_else(|| {
                function
                    .locals
                    .iter()
                    .find(|l| l.id == *id)
                    .map(|l| l.ty.clone())
            }),
        Place::Global(name) => program
            .globals
            .iter()
            .find(|g| g.name == *name)
            .map(|g| g.ty.clone()),
        Place::Field(_, _) | Place::Index(_, _) | Place::Deref(_) => None,
    }
}

// only a local or a global names a type here, so a write through a field or an index escapes it
fn check_function_ptrness(
    function: &AirFunction,
    program: &AirProgram,
    errors: &mut Vec<AirValidationError>,
) {
    if function.is_extern {
        return;
    }
    for block in &function.blocks {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let AirStmtKind::Assign { place, rvalue } = &stmt.kind else {
                continue;
            };
            let (name, want_ptr) = match rvalue {
                Rvalue::AddressOf(_) => ("addr", true),
                Rvalue::Deref(_) => ("deref", false),
                Rvalue::Len(_) => ("len", false),
                _ => continue,
            };
            let Some(ty) = place_type(function, program, place) else {
                continue;
            };
            if matches!(ty, AirType::Ptr(_)) != want_ptr {
                errors.push(AirValidationError {
                    function_name: function.name.clone(),
                    detail: AirValidationDetail::PtrnessMismatch {
                        context: format!("bb{}, stmt #{}", block.id.0, i),
                        rvalue: name,
                        place_ty: format!("{:?}", ty),
                    },
                });
            }
        }
    }
}

fn contains_opaque(ty: &AirType) -> bool {
    match ty {
        AirType::Opaque => true,
        AirType::Ptr(inner)
        | AirType::Array(inner, _)
        | AirType::Slice(inner)
        | AirType::Vec(inner) => contains_opaque(inner),
        AirType::FnPtr { params, ret, .. } => {
            params.iter().any(contains_opaque) || contains_opaque(ret)
        }
        _ => false,
    }
}

fn collect_unknown_enum_names(
    ty: &AirType,
    known_enums: &HashSet<String>,
    missing: &mut Vec<String>,
) {
    match ty {
        AirType::Enum(name) => {
            if !known_enums.contains(name) && !missing.iter().any(|existing| existing == name) {
                missing.push(name.clone());
            }
        }
        AirType::Ptr(inner)
        | AirType::Array(inner, _)
        | AirType::Slice(inner)
        | AirType::Vec(inner) => {
            collect_unknown_enum_names(inner, known_enums, missing);
        }
        AirType::FnPtr { params, ret, .. } => {
            for param in params {
                collect_unknown_enum_names(param, known_enums, missing);
            }
            collect_unknown_enum_names(ret, known_enums, missing);
        }
        _ => {}
    }
}

pub fn validate_air(program: &AirProgram) -> Result<(), Vec<AirValidationError>> {
    let mut errors = Vec::new();
    let known_enums: HashSet<String> = program.enums.iter().map(|def| def.name.clone()).collect();
    let known_globals: HashSet<String> = program.globals.iter().map(|g| g.name.clone()).collect();

    for def in &program.structs {
        for field in &def.fields {
            if contains_opaque(&field.ty) {
                errors.push(AirValidationError {
                    function_name: format!("struct {}", def.name),
                    detail: AirValidationDetail::OpaqueStructField {
                        struct_name: def.name.clone(),
                        field_name: field.name.clone(),
                    },
                });
            }
            let mut missing = Vec::new();
            collect_unknown_enum_names(&field.ty, &known_enums, &mut missing);
            for enum_name in missing {
                errors.push(AirValidationError {
                    function_name: format!("struct {}", def.name),
                    detail: AirValidationDetail::UnknownStructFieldEnum {
                        struct_name: def.name.clone(),
                        field_name: field.name.clone(),
                        enum_name,
                    },
                });
            }
        }
    }

    for function in &program.functions {
        validate_function(function, &known_enums, &known_globals, &mut errors);
        check_function_ptrness(function, program, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_function(
    function: &AirFunction,
    known_enums: &HashSet<String>,
    known_globals: &HashSet<String>,
    errors: &mut Vec<AirValidationError>,
) {
    if function.is_extern {
        return;
    }

    if function.blocks.is_empty() {
        errors.push(AirValidationError {
            function_name: function.name.clone(),
            detail: AirValidationDetail::EmptyBody,
        });
        return;
    }

    let is_void_return = function.ret_ty == AirType::Void;

    for local in &function.locals {
        if local.ty == AirType::Void {
            if is_void_return && local.id == LocalId(0) {
                continue;
            }
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::VoidLocal {
                    local_id: local.id.0,
                    local_name: local.name.clone(),
                },
            });
        }
        // reject opaque types that survived past monomorphization.
        if contains_opaque(&local.ty) {
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::OpaqueType {
                    local_id: local.id.0,
                    local_name: local.name.clone(),
                },
            });
        }
        let mut missing = Vec::new();
        collect_unknown_enum_names(&local.ty, known_enums, &mut missing);
        for enum_name in missing {
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::UnknownEnumType {
                    local_id: local.id.0,
                    local_name: local.name.clone(),
                    enum_name,
                },
            });
        }
    }

    for param in &function.params {
        if param.ty == AirType::Void {
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::VoidLocal {
                    local_id: param.id.0,
                    local_name: Some(param.name.clone()),
                },
            });
        }
        if contains_opaque(&param.ty) {
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::OpaqueType {
                    local_id: param.id.0,
                    local_name: Some(param.name.clone()),
                },
            });
        }
        let mut missing = Vec::new();
        collect_unknown_enum_names(&param.ty, known_enums, &mut missing);
        for enum_name in missing {
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::UnknownEnumType {
                    local_id: param.id.0,
                    local_name: Some(param.name.clone()),
                    enum_name,
                },
            });
        }
    }

    if contains_opaque(&function.ret_ty) {
        errors.push(AirValidationError {
            function_name: function.name.clone(),
            detail: AirValidationDetail::OpaqueType {
                local_id: 0,
                local_name: Some("(return type)".to_string()),
            },
        });
    }
    let mut missing = Vec::new();
    collect_unknown_enum_names(&function.ret_ty, known_enums, &mut missing);
    for enum_name in missing {
        errors.push(AirValidationError {
            function_name: function.name.clone(),
            detail: AirValidationDetail::UnknownEnumType {
                local_id: 0,
                local_name: Some("(return type)".to_string()),
                enum_name,
            },
        });
    }

    let declared_locals: HashSet<LocalId> = function
        .params
        .iter()
        .map(|p| p.id)
        .chain(function.locals.iter().map(|l| l.id))
        .collect();

    let declared_blocks: HashSet<BlockId> = function.blocks.iter().map(|b| b.id).collect();

    for block in &function.blocks {
        let block_ctx = format!("bb{}", block.id.0);
        check_block_locals(
            block,
            &declared_locals,
            known_enums,
            known_globals,
            &function.name,
            &block_ctx,
            errors,
        );
        check_block_target_blocks(block, &declared_blocks, &function.name, &block_ctx, errors);
    }

    check_terminator_operand_writes(function, errors);
}

fn place_base_local(place: &Place) -> Option<LocalId> {
    match place {
        Place::Local(id) | Place::Field(id, _) | Place::Deref(id) | Place::Index(id, _) => Some(*id),
        Place::Global(_) => None,
    }
}

fn collect_written_locals(function: &AirFunction) -> HashSet<LocalId> {
    let mut written: HashSet<LocalId> = function.params.iter().map(|p| p.id).collect();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign { place, rvalue } => {
                    written.extend(place_base_local(place));
                    // an address handed to a callee is an initialization this pass cannot see
                    if let Rvalue::AddressOf(inner) = rvalue {
                        written.extend(place_base_local(inner));
                    }
                }
                AirStmtKind::GcAlloc { local, .. }
                | AirStmtKind::Alloc { local, .. }
                | AirStmtKind::RcAlloc { local, .. } => {
                    written.insert(*local);
                }
                AirStmtKind::GcDrop(_)
                | AirStmtKind::Free(_)
                | AirStmtKind::CallVoid { .. }
                | AirStmtKind::ArenaCreate(_)
                | AirStmtKind::ArenaDestroy(_)
                | AirStmtKind::MemoryFence(_) => {}
            }
        }
        if let AirTerminator::Invoke { ret, .. } = &block.terminator {
            written.extend(place_base_local(ret));
        }
    }
    written
}

fn terminator_targets(term: &AirTerminator) -> Vec<BlockId> {
    match term {
        AirTerminator::Goto(target) => vec![*target],
        AirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        AirTerminator::Switch {
            targets, default, ..
        } => targets
            .iter()
            .map(|(_, target)| *target)
            .chain(std::iter::once(*default))
            .collect(),
        AirTerminator::Invoke { normal, unwind, .. } => vec![*normal, *unwind],
        AirTerminator::Return(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => Vec::new(),
    }
}

fn reachable_blocks(function: &AirFunction) -> HashSet<BlockId> {
    let mut has_predecessors: HashSet<BlockId> = HashSet::new();
    for block in &function.blocks {
        has_predecessors.extend(terminator_targets(&block.terminator));
    }
    let entry = function
        .blocks
        .iter()
        .find(|b| !has_predecessors.contains(&b.id))
        .or_else(|| function.blocks.first())
        .map(|b| b.id);
    let mut reachable = HashSet::new();
    let mut work: Vec<BlockId> = entry.into_iter().collect();
    while let Some(id) = work.pop() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(block) = function.blocks.iter().find(|b| b.id == id) {
            work.extend(terminator_targets(&block.terminator));
        }
    }
    reachable
}

fn check_terminator_operand_writes(function: &AirFunction, errors: &mut Vec<AirValidationError>) {
    let written = collect_written_locals(function);
    let reachable = reachable_blocks(function);
    for block in &function.blocks {
        if !reachable.contains(&block.id) {
            continue;
        }
        let mut operands: Vec<&Operand> = Vec::new();
        match &block.terminator {
            AirTerminator::Return(Some(op)) => operands.push(op),
            AirTerminator::Branch { cond, .. } => operands.push(cond),
            AirTerminator::Switch { discr, .. } => operands.push(discr),
            AirTerminator::Invoke { args, .. } => operands.extend(args.iter()),
            AirTerminator::Return(None)
            | AirTerminator::Goto(_)
            | AirTerminator::Unwind
            | AirTerminator::Unreachable
            | AirTerminator::Panic { .. } => {}
        }
        for op in operands {
            let id = match op {
                Operand::Copy(id) | Operand::Move(id) => *id,
                Operand::Const(_) => continue,
            };
            if written.contains(&id) {
                continue;
            }
            errors.push(AirValidationError {
                function_name: function.name.clone(),
                detail: AirValidationDetail::UnwrittenTerminatorOperand {
                    local_id: id.0,
                    local_name: function
                        .locals
                        .iter()
                        .find(|l| l.id == id)
                        .and_then(|l| l.name.clone()),
                    context: format!("bb{}, terminator", block.id.0),
                },
            });
        }
    }
}

fn check_block_locals(
    block: &AirBlock,
    declared: &HashSet<LocalId>,
    known_enums: &HashSet<String>,
    known_globals: &HashSet<String>,
    func_name: &str,
    block_ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    for (i, stmt) in block.stmts.iter().enumerate() {
        let ctx = format!("{block_ctx}, stmt #{i}");
        check_stmt_locals(
            &stmt.kind,
            declared,
            known_enums,
            known_globals,
            func_name,
            &ctx,
            errors,
        );
    }
    let ctx = format!("{block_ctx}, terminator");
    check_terminator_locals(
        &block.terminator,
        declared,
        known_enums,
        known_globals,
        func_name,
        &ctx,
        errors,
    );
}

fn check_stmt_locals(
    stmt: &AirStmtKind,
    declared: &HashSet<LocalId>,
    known_enums: &HashSet<String>,
    known_globals: &HashSet<String>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match stmt {
        AirStmtKind::Assign { place, rvalue } => {
            check_place_locals(place, declared, known_globals, func_name, ctx, errors);
            check_rvalue_locals(
                rvalue,
                declared,
                known_enums,
                known_globals,
                func_name,
                ctx,
                errors,
            );
        }
        AirStmtKind::GcAlloc { local, .. }
        | AirStmtKind::Alloc { local, .. }
        | AirStmtKind::RcAlloc { local, .. } => {
            check_local(*local, declared, func_name, ctx, errors);
        }
        AirStmtKind::GcDrop(local) | AirStmtKind::Free(local) => {
            check_local(*local, declared, func_name, ctx, errors);
        }
        AirStmtKind::CallVoid { func, args } => {
            check_callee_locals(func, declared, func_name, ctx, errors);
            for arg in args {
                check_operand_locals(arg, declared, func_name, ctx, errors);
            }
        }
        AirStmtKind::ArenaCreate(_)
        | AirStmtKind::ArenaDestroy(_)
        | AirStmtKind::MemoryFence(_) => {}
    }
}

fn check_terminator_locals(
    term: &AirTerminator,
    declared: &HashSet<LocalId>,
    _known_enums: &HashSet<String>,
    known_globals: &HashSet<String>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match term {
        AirTerminator::Return(Some(op)) => {
            check_operand_locals(op, declared, func_name, ctx, errors);
        }
        AirTerminator::Branch { cond, .. } => {
            check_operand_locals(cond, declared, func_name, ctx, errors);
        }
        AirTerminator::Switch { discr, .. } => {
            check_operand_locals(discr, declared, func_name, ctx, errors);
        }
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            check_callee_locals(func, declared, func_name, ctx, errors);
            for arg in args {
                check_operand_locals(arg, declared, func_name, ctx, errors);
            }
            check_place_locals(ret, declared, known_globals, func_name, ctx, errors);
        }
        AirTerminator::Return(None)
        | AirTerminator::Goto(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn check_rvalue_locals(
    rvalue: &Rvalue,
    declared: &HashSet<LocalId>,
    known_enums: &HashSet<String>,
    known_globals: &HashSet<String>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) | Rvalue::Len(op) => {
            check_operand_locals(op, declared, func_name, ctx, errors);
        }
        Rvalue::BinaryOp(_, left, right) => {
            check_operand_locals(left, declared, func_name, ctx, errors);
            check_operand_locals(right, declared, func_name, ctx, errors);
        }
        Rvalue::Call { func, args } => {
            check_callee_locals(func, declared, func_name, ctx, errors);
            for arg in args {
                check_operand_locals(arg, declared, func_name, ctx, errors);
            }
        }
        Rvalue::StructInit { fields, .. } => {
            for (_, operand) in fields {
                check_operand_locals(operand, declared, func_name, ctx, errors);
            }
        }
        Rvalue::FieldAccess { base, .. } | Rvalue::Cast { operand: base, .. } => {
            check_operand_locals(base, declared, func_name, ctx, errors);
        }
        Rvalue::Index { base, index } => {
            check_operand_locals(base, declared, func_name, ctx, errors);
            check_operand_locals(index, declared, func_name, ctx, errors);
        }
        Rvalue::AddressOf(place) => {
            check_place_locals(place, declared, known_globals, func_name, ctx, errors);
        }
        Rvalue::EnumInit {
            enum_name, payload, ..
        } => {
            if !known_enums.contains(enum_name) {
                errors.push(AirValidationError {
                    function_name: func_name.to_string(),
                    detail: AirValidationDetail::UnknownEnumReference {
                        enum_name: enum_name.clone(),
                        context: ctx.to_string(),
                    },
                });
            }
            for operand in payload {
                check_operand_locals(operand, declared, func_name, ctx, errors);
            }
        }
        Rvalue::EnumTag { enum_name, operand } => {
            if !known_enums.contains(enum_name) {
                errors.push(AirValidationError {
                    function_name: func_name.to_string(),
                    detail: AirValidationDetail::UnknownEnumReference {
                        enum_name: enum_name.clone(),
                        context: ctx.to_string(),
                    },
                });
            }
            check_operand_locals(operand, declared, func_name, ctx, errors);
        }
        Rvalue::EnumPayload {
            enum_name, operand, ..
        } => {
            if !known_enums.contains(enum_name) {
                errors.push(AirValidationError {
                    function_name: func_name.to_string(),
                    detail: AirValidationDetail::UnknownEnumReference {
                        enum_name: enum_name.clone(),
                        context: ctx.to_string(),
                    },
                });
            }
            check_operand_locals(operand, declared, func_name, ctx, errors);
        }
        Rvalue::ClosureCreate { env, .. } => {
            check_operand_locals(env, declared, func_name, ctx, errors);
        }
        Rvalue::SliceFromParts { ptr, len } => {
            check_operand_locals(ptr, declared, func_name, ctx, errors);
            check_operand_locals(len, declared, func_name, ctx, errors);
        }
    }
}

fn check_place_locals(
    place: &Place,
    declared: &HashSet<LocalId>,
    known_globals: &HashSet<String>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match place {
        Place::Global(name) => {
            if !known_globals.contains(name) {
                errors.push(AirValidationError {
                    function_name: func_name.to_string(),
                    detail: AirValidationDetail::UnknownGlobalReference {
                        global_name: name.clone(),
                        context: ctx.to_string(),
                    },
                });
            }
        }
        Place::Local(local) | Place::Field(local, _) | Place::Deref(local) => {
            check_local(*local, declared, func_name, ctx, errors);
        }
        Place::Index(local, operand) => {
            check_local(*local, declared, func_name, ctx, errors);
            check_operand_locals(operand, declared, func_name, ctx, errors);
        }
    }
}

fn check_operand_locals(
    operand: &Operand,
    declared: &HashSet<LocalId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => {
            check_local(*local, declared, func_name, ctx, errors);
        }
        Operand::Const(_) => {}
    }
}

fn check_callee_locals(
    callee: &Callee,
    declared: &HashSet<LocalId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    if let Callee::FnPtr(local) = callee {
        check_local(*local, declared, func_name, ctx, errors);
    }
}

fn check_local(
    local: LocalId,
    declared: &HashSet<LocalId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    if !declared.contains(&local) {
        errors.push(AirValidationError {
            function_name: func_name.to_string(),
            detail: AirValidationDetail::UndeclaredLocal {
                local_id: local.0,
                context: ctx.to_string(),
            },
        });
    }
}

fn check_block_target_blocks(
    block: &AirBlock,
    declared: &HashSet<BlockId>,
    func_name: &str,
    block_ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    let ctx = format!("{block_ctx}, terminator");
    match &block.terminator {
        AirTerminator::Goto(target) => {
            check_block_ref(*target, declared, func_name, &ctx, errors);
        }
        AirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            check_block_ref(*then_block, declared, func_name, &ctx, errors);
            check_block_ref(*else_block, declared, func_name, &ctx, errors);
        }
        AirTerminator::Switch {
            targets, default, ..
        } => {
            for (_, target) in targets {
                check_block_ref(*target, declared, func_name, &ctx, errors);
            }
            check_block_ref(*default, declared, func_name, &ctx, errors);
        }
        AirTerminator::Invoke { normal, unwind, .. } => {
            check_block_ref(*normal, declared, func_name, &ctx, errors);
            check_block_ref(*unwind, declared, func_name, &ctx, errors);
        }
        AirTerminator::Return(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn check_block_ref(
    block: BlockId,
    declared: &HashSet<BlockId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    if !declared.contains(&block) {
        errors.push(AirValidationError {
            function_name: func_name.to_string(),
            detail: AirValidationDetail::UndeclaredBlock {
                block_id: block.0,
                context: ctx.to_string(),
            },
        });
    }
}
