use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::ast::{Function, StmtKind};

pub(super) struct UntypedBodyResult {
    pub(super) returned: bool,
}

impl UntypedBodyResult {
    fn returned() -> Self {
        Self { returned: true }
    }

    fn not_returned() -> Self {
        Self { returned: false }
    }
}

pub(super) fn compile_untyped_body(
    func_compiler: &mut Compiler,
    func: &Function,
    has_no_gc: bool,
) -> Result<UntypedBodyResult> {
    if func.body.is_empty() {
        return Ok(UntypedBodyResult::not_returned());
    }

    for stmt in &func.body[..func.body.len() - 1] {
        func_compiler.compile_stmt(stmt)?;
    }

    let last_stmt = &func.body[func.body.len() - 1];

    match &last_stmt.kind {
        StmtKind::Expression(expr) => {
            let result_reg = func_compiler.alloc_register()?;
            func_compiler.compile_expr(expr, result_reg)?;

            if has_no_gc {
                let line = func_compiler.current_line(func.span);
                func_compiler
                    .current
                    .emit_a(OpCode::ExitNoGc, 0, 0, 0, line);
            }

            let line = func_compiler.current_line(last_stmt.span);
            func_compiler
                .current
                .emit_a(OpCode::Return, result_reg, 0, 0, line);
            Ok(UntypedBodyResult::returned())
        }
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } if else_branch.is_some() => {
            let result_reg = func_compiler.alloc_register()?;
            let cond_reg = func_compiler.alloc_register()?;

            func_compiler.compile_expr(condition, cond_reg)?;
            let jump_to_else =
                func_compiler.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
            func_compiler.free_register(cond_reg);

            func_compiler.compile_if_branch_for_return(then_branch, result_reg)?;
            let jump_to_end = func_compiler.emit_jump(OpCode::Jump, then_branch.span);
            func_compiler.patch_jump(jump_to_else);

            if let Some(else_branch) = else_branch.as_ref() {
                func_compiler.compile_if_branch_for_return(else_branch, result_reg)?;
            }
            func_compiler.patch_jump(jump_to_end);

            if has_no_gc {
                let line = func_compiler.current_line(func.span);
                func_compiler
                    .current
                    .emit_a(OpCode::ExitNoGc, 0, 0, 0, line);
            }

            let line = func_compiler.current_line(last_stmt.span);
            func_compiler
                .current
                .emit_a(OpCode::Return, result_reg, 0, 0, line);

            Ok(UntypedBodyResult::returned())
        }
        _ => {
            func_compiler.compile_stmt(last_stmt)?;
            Ok(UntypedBodyResult::not_returned())
        }
    }
}
