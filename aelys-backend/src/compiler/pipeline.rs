use super::Compiler;
use aelys_bytecode::{Function, GlobalLayout, Heap, OpCode};
use aelys_common::Result;
use aelys_sema::{TypedProgram, TypedStmtKind};
use std::collections::HashMap;
use std::sync::Arc;

impl Compiler {
    pub fn compile_typed(
        mut self,
        program: &TypedProgram,
    ) -> Result<(Function, Heap, HashMap<String, bool>)> {
        for stmt in &program.stmts {
            match &stmt.kind {
                TypedStmtKind::Function(func) => {
                    self.globals.insert(func.name.clone(), false);
                    if !self.global_indices.contains_key(&func.name) {
                        let idx = self.next_global_index;
                        self.global_indices.insert(func.name.clone(), idx);
                        self.next_global_index += 1;
                    }
                }
                TypedStmtKind::Let { name, mutable, .. } => {
                    self.globals.insert(name.clone(), *mutable);
                    if !self.global_indices.contains_key(name) {
                        let idx = self.next_global_index;
                        self.global_indices.insert(name.clone(), idx);
                        self.next_global_index += 1;
                    }
                }
                _ => {}
            }
        }

        if program.stmts.is_empty() {
            self.emit_return0(aelys_syntax::Span::dummy());
        } else {
            let last_idx = program.stmts.len() - 1;

            for stmt in &program.stmts[..last_idx] {
                self.compile_typed_stmt(stmt)?;
            }

            let last_stmt = &program.stmts[last_idx];
            match &last_stmt.kind {
                TypedStmtKind::Expression(expr) => {
                    let result_reg = self.alloc_register()?;
                    self.compile_typed_expr(expr, result_reg)?;
                    self.emit_a(OpCode::Return, result_reg, 0, 0, last_stmt.span);
                }
                _ => {
                    self.compile_typed_stmt(last_stmt)?;
                    self.emit_return0(last_stmt.span);
                }
            }
        }

        self.current.num_registers = self.next_register;
        self.current.call_site_count = self.next_call_site_slot;
        self.current.global_layout = self.build_global_layout();
        self.current.compute_global_layout_hash();
        self.current.finalize_bytecode();

        Ok((self.current, self.heap, self.globals))
    }

    pub(super) fn build_global_layout(&self) -> Arc<GlobalLayout> {
        if self.accessed_globals.is_empty() {
            GlobalLayout::empty()
        } else {
            let mut names = vec![String::new(); self.next_global_index as usize];
            for (name, &idx) in &self.global_indices {
                if self.accessed_globals.contains(name) {
                    names[idx as usize] = name.clone();
                }
            }
            GlobalLayout::new(names)
        }
    }
}
