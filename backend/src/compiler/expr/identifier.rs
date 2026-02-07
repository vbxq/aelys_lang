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
            // For direct imports, use the qualified name in global_layout
            // so bytecode loading can detect which module to load
            let actual_name =
                if self.known_globals.contains(name) && !self.globals.contains_key(name) {
                    self.resolve_global_name(name).to_string()
                } else {
                    name.to_string()
                };

            let idx = if let Some(&idx) = self.global_indices.get(&actual_name) {
                idx
            } else if self.globals.contains_key(name)
                || Self::is_builtin(name)
                || self.known_globals.contains(name)
            {
                let idx = self.next_global_index;
                self.global_indices.insert(actual_name.clone(), idx);
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
            self.accessed_globals.insert(actual_name);
            self.emit_b(OpCode::GetGlobalIdx, dest, idx as i16, span);
            Ok(())
        }
    }

    pub fn get_local_register(&self, expr: &Expr) -> Option<u8> {
        if let ExprKind::Identifier(name) = &expr.kind
            && let Some((reg, _)) = self.resolve_variable(name)
        {
            return Some(reg);
        }
        None
    }
}
