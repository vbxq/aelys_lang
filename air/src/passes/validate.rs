//! AIR validation pass
//!
//! checks structural invariants of the AIR program before codegen
//!
//! this pass does not mutate the program, we just inspects it and
//! collects all violations found
//!
//! some stuff enforced:
//!
//! - no `AirType::Void` on a local unless it is the return position of a void returning function (i.e. only the implicit, so `_0` return local of a `ret_ty == Void` function may be Void)
//! - eo `AirType::Opaque` anywhere, because this means an unresolved Dynamic type survived past monomorphization
//! - every basic block has a structurally valid terminator, guaranteed by construction, but we double-check !
//! - every local referenced in operands/places is declared in the function's params or locals list
//! - every block referenced by terminators exists in the function.

use crate::{
    AirBlock, AirFunction, AirProgram, AirStmtKind, AirTerminator, AirType, BlockId, Callee,
    LocalId, Operand, Place, Rvalue,
};
use std::collections::HashSet;
use std::fmt;

/// A single validation error with context about where it was found.
#[derive(Debug, Clone)]
pub struct AirValidationError {
    pub function_name: String,
    pub detail: AirValidationDetail,
}

#[derive(Debug, Clone)]
pub enum AirValidationDetail {
    /// A local has type Void but is not the return local of a void function.
    VoidLocal {
        local_id: u32,
        local_name: Option<String>,
    },
    /// A local referenced in the body is not declared.
    UndeclaredLocal { local_id: u32, context: String },
    /// A block referenced by a terminator does not exist.
    UndeclaredBlock { block_id: u32, context: String },
    /// A local or param has type Opaque (unresolved Dynamic that survived monomorphization).
    OpaqueType {
        local_id: u32,
        local_name: Option<String>,
    },
    /// A struct field has type Opaque.
    OpaqueStructField {
        struct_name: String,
        field_name: String,
    },
    /// A local or param references an enum definition that is not present in the AIR program.
    UnknownEnumType {
        local_id: u32,
        local_name: Option<String>,
        enum_name: String,
    },
    /// A struct field references an enum definition that is not present in the AIR program.
    UnknownStructFieldEnum {
        struct_name: String,
        field_name: String,
        enum_name: String,
    },
    /// An enum operation references an enum definition that is not present in the AIR program.
    UnknownEnumReference { enum_name: String, context: String },
    /// a place names a global that is not present in the air program.
    UnknownGlobalReference {
        global_name: String,
        context: String,
    },
    /// A function has no blocks (non-extern function with empty body).
    EmptyBody,
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

/// Returns true if the type contains Opaque anywhere (including nested in compound types like Array, Ptr, FnPtr, Slice)
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

/// Validate the entire AIR program. Returns `Ok(())` if all invariants hold, or `Err(errors)` with every violation found
pub fn validate_air(program: &AirProgram) -> Result<(), Vec<AirValidationError>> {
    let mut errors = Vec::new();
    let known_enums: HashSet<String> = program.enums.iter().map(|def| def.name.clone()).collect();
    let known_globals: HashSet<String> = program.globals.iter().map(|g| g.name.clone()).collect();

    // Check struct fields for Opaque types.
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
    // Skip extern declarations, they have no body by design.
    if function.is_extern {
        return;
    }

    // non-extern function must have at least one block check
    if function.blocks.is_empty() {
        errors.push(AirValidationError {
            function_name: function.name.clone(),
            detail: AirValidationDetail::EmptyBody,
        });
        // No point checking locals/blocks if the body is empty
        return;
    }

    // no Void-typed locals except void return position check
    //
    // Convention: the return local is local %0 when ret_ty != Void
    //
    // When ret_ty == Void, local %0 may be Void (it's the implicit return slot)
    //
    // Any other local with Void type is a bug.
    let is_void_return = function.ret_ty == AirType::Void;

    for local in &function.locals {
        if local.ty == AirType::Void {
            // Allow the return local (%0) of a void-returning function
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
        // Reject Opaque types that survived past monomorphization.
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

    // Also check params for Void and Opaque types.
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

    // check return type for Opaque.
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

    // build declared-locals and declared-blocks sets
    let declared_locals: HashSet<LocalId> = function
        .params
        .iter()
        .map(|p| p.id)
        .chain(function.locals.iter().map(|l| l.id))
        .collect();

    let declared_blocks: HashSet<BlockId> = function.blocks.iter().map(|b| b.id).collect();

    // all referenced locals exist check
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

// block reference checking
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

