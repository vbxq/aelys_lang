use super::super::Compiler;
use super::finalize::finalize_lambda;
use aelys_bytecode::{Heap, OpCode};
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Parameter, Stmt, StmtKind};

impl Compiler {
    pub fn compile_lambda(
        &mut self,
        params: &[Parameter],
        body: &[Stmt],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let heap = std::mem::replace(&mut self.heap, Heap::new());
        let globals = self.globals.clone();
        let global_indices = self.global_indices.clone();
        let enclosing_locals = self.locals.clone();
        let enclosing_upvalues = self.upvalues.clone();

        let mut lambda_compiler = Compiler::for_nested_function(
            None,
            self.source.clone(),
            heap,
            globals,
            global_indices,
            self.next_global_index,
            enclosing_locals,
            enclosing_upvalues,
            self.all_enclosing_locals.clone(),
            self.module_aliases.clone(),
            self.known_globals.clone(),
            self.known_native_globals.clone(),
            self.next_call_site_slot,
        );

        lambda_compiler.begin_scope();

        for param in params {
            lambda_compiler.declare_variable(&param.name, false)?;
        }

        compile_lambda_body(&mut lambda_compiler, body, span)?;

        lambda_compiler.end_scope();
        lambda_compiler.current.num_registers = lambda_compiler.next_register;
        lambda_compiler.current.arity = params.len() as u8;

        finalize_lambda(self, lambda_compiler, dest, span)
    }
}

fn compile_lambda_body(lambda_compiler: &mut Compiler, body: &[Stmt], span: Span) -> Result<()> {
    if body.is_empty() {
        lambda_compiler.emit_return0(span);
        return Ok(());
    }

    for stmt in &body[..body.len() - 1] {
        lambda_compiler.compile_stmt(stmt)?;
    }

    let last_stmt = &body[body.len() - 1];

    match &last_stmt.kind {
        StmtKind::Expression(expr) => {
            let result_reg = lambda_compiler.alloc_register()?;
            lambda_compiler.compile_expr(expr, result_reg)?;
            lambda_compiler.emit_a(OpCode::Return, result_reg, 0, 0, last_stmt.span);
        }
        _ => {
            lambda_compiler.compile_stmt(last_stmt)?;
            lambda_compiler.emit_return0(span);
        }
    }

    Ok(())
}
