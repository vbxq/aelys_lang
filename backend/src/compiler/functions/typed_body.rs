use super::super::Compiler;
use super::super::liveness::LivenessAnalysis;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_sema::{TypedFunction, TypedStmtKind};
use std::collections::HashSet;

pub(super) fn compile_typed_body(
    nested_compiler: &mut Compiler,
    func: &TypedFunction,
    has_no_gc: bool,
) -> Result<()> {
    nested_compiler.begin_scope();

    for param in &func.params {
        let reg = nested_compiler.alloc_register()?;
        let resolved_type = aelys_sema::ResolvedType::from_infer_type(&param.ty);
        nested_compiler.add_local(param.name.clone(), false, reg, resolved_type);
    }

    if has_no_gc {
        nested_compiler.emit_a(OpCode::EnterNoGc, 0, 0, 0, func.span);
    }

    let liveness = LivenessAnalysis::analyze_function(func);

    if func.body.is_empty() {
        if has_no_gc {
            nested_compiler.emit_a(OpCode::ExitNoGc, 0, 0, 0, func.span);
        }
        nested_compiler.emit_a(OpCode::Return0, 0, 0, 0, func.span);
        nested_compiler.end_scope();
        return Ok(());
    }

    let last_idx = func.body.len() - 1;
    let mut already_freed = HashSet::new();
    for (stmt_idx, stmt) in func.body[..last_idx].iter().enumerate() {
        nested_compiler.compile_typed_stmt(stmt)?;
        nested_compiler.free_dead_locals(stmt_idx, &liveness, &mut already_freed);
    }

    let last_stmt = &func.body[last_idx];

    let implicit_return_reg = match &last_stmt.kind {
        TypedStmtKind::Expression(expr) => {
            let result_reg = nested_compiler.alloc_register()?;
            nested_compiler.compile_typed_expr(expr, result_reg)?;
            Some(result_reg)
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } if else_branch.is_some() => {
            let result_reg = nested_compiler.alloc_register()?;
            let cond_reg = nested_compiler.alloc_register()?;

            nested_compiler.compile_typed_expr(condition, cond_reg)?;
            let jump_to_else =
                nested_compiler.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
            nested_compiler.free_register(cond_reg);

            nested_compiler.compile_typed_if_branch_for_return(then_branch, result_reg)?;
            let jump_to_end = nested_compiler.emit_jump(OpCode::Jump, then_branch.span);
            nested_compiler.patch_jump(jump_to_else);

            if let Some(else_branch) = else_branch.as_ref() {
                nested_compiler.compile_typed_if_branch_for_return(else_branch, result_reg)?;
            }
            nested_compiler.patch_jump(jump_to_end);

            Some(result_reg)
        }
        _ => {
            nested_compiler.compile_typed_stmt(last_stmt)?;
            nested_compiler.free_dead_locals(last_idx, &liveness, &mut already_freed);
            None
        }
    };

    if has_no_gc {
        nested_compiler.emit_a(OpCode::ExitNoGc, 0, 0, 0, func.span);
    }

    if let Some(result_reg) = implicit_return_reg {
        nested_compiler.emit_a(OpCode::Return, result_reg, 0, 0, func.span);
    } else {
        nested_compiler.emit_a(OpCode::Return0, 0, 0, 0, func.span);
    }

    nested_compiler.end_scope();
    Ok(())
}
