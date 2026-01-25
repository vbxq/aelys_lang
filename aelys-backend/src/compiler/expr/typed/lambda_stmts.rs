use super::super::super::Upvalue;
use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_lambda_with_stmts(
        &mut self,
        params: &[aelys_sema::TypedParam],
        body: &[aelys_sema::TypedStmt],
        captures: &[(String, aelys_sema::InferType)],
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
            self.next_call_site_slot,
        );
        nested_compiler.current.arity = params.len() as u8;

        for (capture_name, _capture_ty) in captures {
            if let Some((local_reg, mutable, _resolved_ty)) =
                self.resolve_variable_typed(capture_name)
            {
                if !nested_compiler
                    .upvalues
                    .iter()
                    .any(|uv| uv.name == *capture_name)
                {
                    self.mark_local_captured(local_reg);
                    nested_compiler.upvalues.push(Upvalue {
                        is_local: true,
                        index: local_reg,
                        name: capture_name.clone(),
                        mutable,
                    });
                }
                continue;
            }

            let _ = self.resolve_upvalue(capture_name);

            if let Some((upvalue_idx, _)) = self
                .upvalues
                .iter()
                .enumerate()
                .find(|(_, uv)| uv.name == *capture_name)
            {
                if !nested_compiler
                    .upvalues
                    .iter()
                    .any(|uv| uv.name == *capture_name)
                {
                    nested_compiler.upvalues.push(Upvalue {
                        is_local: false,
                        index: upvalue_idx as u8,
                        name: capture_name.clone(),
                        mutable: self.upvalues[upvalue_idx].mutable,
                    });
                }
            }
        }

        nested_compiler.begin_scope();

        for param in params {
            let reg = nested_compiler.alloc_register()?;
            let resolved_type = aelys_sema::ResolvedType::from_infer_type(&param.ty);
            nested_compiler.add_local(param.name.clone(), false, reg, resolved_type);
        }

        if !body.is_empty() {
            for stmt in &body[..body.len() - 1] {
                nested_compiler.compile_typed_stmt(stmt)?;
            }

            let last_stmt = &body[body.len() - 1];
            match &last_stmt.kind {
                aelys_sema::TypedStmtKind::Expression(expr) => {
                    let result_reg = nested_compiler.alloc_register()?;
                    nested_compiler.compile_typed_expr(expr, result_reg)?;
                    nested_compiler.emit_a(OpCode::Return, result_reg, 0, 0, last_stmt.span);
                }
                aelys_sema::TypedStmtKind::Return(_) => {
                    nested_compiler.compile_typed_stmt(last_stmt)?;
                }
                _ => {
                    nested_compiler.compile_typed_stmt(last_stmt)?;
                    nested_compiler.emit_return0(last_stmt.span);
                }
            }
        } else {
            nested_compiler.emit_return0(span);
        }

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

        let const_idx = self.current.add_constant_function(compiled_func);

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
        }

        Ok(())
    }
}
