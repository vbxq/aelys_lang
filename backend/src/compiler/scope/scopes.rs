use super::super::{Compiler, Scope};
use aelys_bytecode::OpCode;

impl Compiler {
    pub fn begin_scope(&mut self) {
        self.scope_depth += 1;
        self.scopes.push(Scope {
            start: self.locals.len(),
            captured_registers: Vec::new(),
        });
    }

    // End scope: close upvalues for captured variables, free registers.
    // Note: we only emit CloseUpvals if something was actually captured.
    // Earlier versions emitted it unconditionally which was wasteful
    pub fn end_scope(&mut self) {
        self.scope_depth = self.scope_depth.saturating_sub(1);

        if let Some(scope) = self.scopes.pop() {
            // Only need to close upvalues if any locals were captured
            if let Some(&lowest_captured) = scope.captured_registers.iter().min() {
                self.current
                    .emit_a(OpCode::CloseUpvals, lowest_captured, 0, 0, 0);
            }

            for local in self.locals.drain(scope.start..) {
                self.register_pool[local.register as usize] = false;
            }
        }
    }
}
