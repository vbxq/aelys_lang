use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    // Variable resolution order: local -> upvalue -> global.
    // Note: we used to emit GetGlobal (name-based lookup) for globals,
    // now we use GetGlobalIdx (index-based) for better performance
    pub fn compile_identifier(&mut self, name: &str, dest: u8, span: Span) -> Result<()> {
        // Local variable - just move from its register (or skip if already in dest)
        if let Some((reg, _mutable)) = self.resolve_variable(name) {
            if reg != dest {
                self.emit_a(OpCode::Move, dest, reg, 0, span);
            }
            Ok(())
        } else if let Some((upvalue_idx, _mutable)) = self.resolve_upvalue(name) {
            self.emit_a(OpCode::GetUpval, dest, upvalue_idx, 0, span);
            Ok(())
        } else {
            let idx = if let Some(&idx) = self.global_indices.get(name) {
                idx
            } else if self.globals.contains_key(name)
                || Self::is_builtin(name)
                || self.known_globals.contains(name)
            {
                // honestly i'm surprised that you're going through this code to read what it does
                // read if cute
                let idx = self.next_global_index;
                self.global_indices.insert(name.to_string(), idx);
                self.next_global_index += 1;
                idx
            } else {
                let hint = self.generate_undefined_variable_hint(name);
                let error_msg = if let Some(hint_msg) = hint {
                    format!("{}\n\nhint: {}", name, hint_msg)
                } else {
                    name.to_string()
                };
                return Err(CompileError::new(
                    CompileErrorKind::UndefinedVariable(error_msg),
                    span,
                    self.source.clone(),
                )
                .into());
            };
            self.accessed_globals.insert(name.to_string());
            self.emit_b(OpCode::GetGlobalIdx, dest, idx as i16, span);
            Ok(())
        }
    }

    pub fn get_local_register(&self, expr: &Expr) -> Option<u8> {
        if let ExprKind::Identifier(name) = &expr.kind {
            if let Some((reg, _)) = self.resolve_variable(name) {
                return Some(reg);
            }
        }
        None
    }
}
