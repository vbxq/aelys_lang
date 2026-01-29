use super::super::Compiler;
use aelys_bytecode::{OpCode, Value};
use aelys_common::Result;
use aelys_syntax::Span;

impl Compiler {
    pub fn compile_typed_lambda(
        &mut self,
        params: &[aelys_sema::TypedParam],
        body: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_lambda_impl(params, body, dest, span)
    }

    fn compile_typed_lambda_impl(
        &mut self,
        params: &[aelys_sema::TypedParam],
        body: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let mut nested_compiler = super::super::Compiler::for_nested_function(
            Some("<lambda>".to_string()),
            self.source.clone(),
            self.heap.clone(),
            self.globals.clone(),
            self.global_indices.clone(),
            self.next_global_index,
            self.locals.clone(),
            self.upvalues.clone(),
            self.all_enclosing_locals.clone(),
            self.module_aliases.clone(),
            self.known_globals.clone(),
            self.known_native_globals.clone(),
            self.symbol_origins.clone(),
            self.next_call_site_slot,
        );
        nested_compiler.current.arity = params.len() as u8;

        nested_compiler.begin_scope();

        for param in params {
            let reg = nested_compiler.alloc_register()?;
            let resolved_type = aelys_sema::ResolvedType::from_infer_type(&param.ty);
            nested_compiler.add_local(param.name.clone(), false, reg, resolved_type);
        }

        let result_reg = nested_compiler.alloc_register()?;
        nested_compiler.compile_typed_expr(body, result_reg)?;
        nested_compiler.emit_a(OpCode::Return, result_reg, 0, 0, body.span);

        nested_compiler.end_scope();
        nested_compiler.current.num_registers = nested_compiler.next_register;
        nested_compiler.current.global_layout = nested_compiler.build_global_layout();
        nested_compiler.current.compute_global_layout_hash();

        self.mark_captures_from_nested(&nested_compiler);

        let mut compiled_func = nested_compiler.current.clone();
        let mut nested_upvalues = nested_compiler.upvalues.clone();

        self.fix_transitive_captures(&mut nested_upvalues);

        for upvalue in &nested_upvalues {
            compiled_func
                .upvalue_descriptors
                .push(aelys_bytecode::UpvalueDescriptor {
                    is_local: upvalue.is_local,
                    index: upvalue.index,
                });
        }

        let remap = self.heap.merge(&mut nested_compiler.heap);
        compiled_func.remap_constants(&remap);

        for (name, idx) in &nested_compiler.global_indices {
            if !self.global_indices.contains_key(name) {
                self.global_indices.insert(name.clone(), *idx);
            }
        }
        self.next_global_index = nested_compiler.next_global_index;

        if nested_compiler.next_call_site_slot > self.next_call_site_slot {
            self.next_call_site_slot = nested_compiler.next_call_site_slot;
        }

        let func_ref = self.heap.alloc_function(compiled_func);
        let const_idx = self.add_constant(Value::ptr(func_ref.index()), span)?;

        if nested_upvalues.is_empty() {
            self.emit_b(OpCode::LoadK, dest, const_idx as i16, span);
        } else {
            self.emit_a(
                OpCode::MakeClosure,
                dest,
                const_idx as u8,
                nested_upvalues.len() as u8,
                span,
            );

            for upval in &nested_upvalues {
                self.current
                    .push_raw(((upval.is_local as u32) << 8) | (upval.index as u32));
            }
        }

        Ok(())
    }
}
