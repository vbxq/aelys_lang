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
    /// A function has no blocks (non-extern function with empty body).
    EmptyBody,
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
            AirValidationDetail::EmptyBody => {
                write!(f, "non-extern function has no basic blocks")
            }
        }
    }
}

/// Returns true if the type contains Opaque anywhere (including nested in compound types like Array, Ptr, FnPtr, Slice)
fn contains_opaque(ty: &AirType) -> bool {
    match ty {
        AirType::Opaque => true,
        AirType::Ptr(inner) | AirType::Array(inner, _) | AirType::Slice(inner) => {
            contains_opaque(inner)
        }
        AirType::FnPtr { params, ret, .. } => {
            params.iter().any(contains_opaque) || contains_opaque(ret)
        }
        _ => false,
    }
}

/// Validate the entire AIR program. Returns `Ok(())` if all invariants hold, or `Err(errors)` with every violation found
pub fn validate_air(program: &AirProgram) -> Result<(), Vec<AirValidationError>> {
    let mut errors = Vec::new();

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
        }
    }

    for function in &program.functions {
        validate_function(function, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_function(function: &AirFunction, errors: &mut Vec<AirValidationError>) {
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
        check_block_locals(block, &declared_locals, &function.name, &block_ctx, errors);
        check_block_target_blocks(block, &declared_blocks, &function.name, &block_ctx, errors);
    }
}

fn check_block_locals(
    block: &AirBlock,
    declared: &HashSet<LocalId>,
    func_name: &str,
    block_ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    for (i, stmt) in block.stmts.iter().enumerate() {
        let ctx = format!("{block_ctx}, stmt #{i}");
        check_stmt_locals(&stmt.kind, declared, func_name, &ctx, errors);
    }
    let ctx = format!("{block_ctx}, terminator");
    check_terminator_locals(&block.terminator, declared, func_name, &ctx, errors);
}

fn check_stmt_locals(
    stmt: &AirStmtKind,
    declared: &HashSet<LocalId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match stmt {
        AirStmtKind::Assign { place, rvalue } => {
            check_place_locals(place, declared, func_name, ctx, errors);
            check_rvalue_locals(rvalue, declared, func_name, ctx, errors);
        }
        AirStmtKind::GcAlloc { local, .. } | AirStmtKind::Alloc { local, .. } => {
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
            check_place_locals(ret, declared, func_name, ctx, errors);
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
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => {
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
        Rvalue::AddressOf(local) => {
            check_local(*local, declared, func_name, ctx, errors);
        }
    }
}

fn check_place_locals(
    place: &Place,
    declared: &HashSet<LocalId>,
    func_name: &str,
    ctx: &str,
    errors: &mut Vec<AirValidationError>,
) {
    match place {
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
