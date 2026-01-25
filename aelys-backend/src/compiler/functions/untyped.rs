use super::super::Compiler;
use super::untyped_body::compile_untyped_body;
use super::untyped_finalize::finalize_untyped_function;
use aelys_bytecode::{Heap, OpCode};
use aelys_common::Result;

impl Compiler {
    pub fn compile_function(&mut self, func: &aelys_syntax::ast::Function) -> Result<()> {
        let func_var_reg = self.declare_variable(&func.name, false)?;

        if self.scope_depth == 0 {
            self.globals.insert(func.name.clone(), false);
            if !self.global_indices.contains_key(&func.name) {
                let idx = self.next_global_index;
                self.global_indices.insert(func.name.clone(), idx);
                self.next_global_index += 1;
            }
        }

        let has_no_gc = func.decorators.iter().any(|d| d.name == "no_gc");

        let heap = std::mem::replace(&mut self.heap, Heap::new());
        let globals = self.globals.clone();
        let global_indices = self.global_indices.clone();
        let enclosing_locals = self.locals.clone();
        let enclosing_upvalues = self.upvalues.clone();

        let mut func_compiler = Compiler::for_nested_function(
            Some(func.name.clone()),
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

        func_compiler.has_no_gc = has_no_gc;
        func_compiler.begin_scope();

        for param in &func.params {
            func_compiler.declare_variable(&param.name, false)?;
        }

        if has_no_gc {
            let line = func_compiler.current_line(func.span);
            func_compiler
                .current
                .emit_a(OpCode::EnterNoGc, 0, 0, 0, line);
        }

        let body_result = compile_untyped_body(&mut func_compiler, func, has_no_gc)?;

        func_compiler.end_scope();

        if !body_result.returned {
            if has_no_gc {
                let line = func_compiler.current_line(func.span);
                func_compiler
                    .current
                    .emit_a(OpCode::ExitNoGc, 0, 0, 0, line);
            }
            func_compiler.emit_return0(func.span);
        }

        func_compiler.current.num_registers = func_compiler.next_register;
        func_compiler.current.arity = func.params.len() as u8;

        finalize_untyped_function(self, func_compiler, &func.name, func.span, func_var_reg)
    }
}
