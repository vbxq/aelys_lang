use super::Compiler;
use aelys_bytecode::{OpCode, Value};
use aelys_common::Result;
use aelys_syntax::Span;

// bytecode emission wrappers - add line info for debug

impl Compiler {
    #[inline]
    pub fn current_line(&self, span: Span) -> u32 {
        span.line
    }

    // format A: op | a | b | c (3 regs)
    pub fn emit_a(&mut self, op: OpCode, a: u8, b: u8, c: u8, span: Span) {
        self.current.emit_a(op, a, b, c, self.current_line(span));
    }

    // format B: op | a | imm16
    pub fn emit_b(&mut self, op: OpCode, a: u8, imm: i16, span: Span) {
        self.current.emit_b(op, a, imm, self.current_line(span));
    }

    pub fn emit_c(&mut self, op: OpCode, dest: u8, func: u8, nargs: u8, span: Span) {
        self.current
            .emit_c(op, dest, func, nargs, self.current_line(span));
    }

    pub fn emit_jump(&mut self, op: OpCode, span: Span) -> usize {
        self.current.emit_jump(op, self.current_line(span))
    }

    pub fn emit_jump_if(&mut self, op: OpCode, reg: u8, span: Span) -> usize {
        self.current.emit_jump_if(op, reg, self.current_line(span))
    }

    pub fn patch_jump(&mut self, offset: usize) {
        self.current.patch_jump(offset);
    }

    pub fn emit_return0(&mut self, span: Span) {
        self.current
            .emit_a(OpCode::Return0, 0, 0, 0, self.current_line(span));
    }

    pub fn current_offset(&self) -> usize {
        self.current.current_offset()
    }

    pub fn add_constant(&mut self, value: Value, _span: Span) -> Result<u16> {
        let idx = self.current.add_constant(value);
        Ok(idx)
    }

    // inline cache: [op|dest|idx|nargs] [cache_lo] [cache_hi|slot_id]
    // known natives skip runtime patching
    pub fn emit_call_global_cached(
        &mut self,
        dest: u8,
        global_idx: u8,
        nargs: u8,
        global_name: &str,
        span: Span,
    ) {
        let line = self.current_line(span);

        // Check if this is a known native function (builtin or stdlib)
        let is_known_native =
            Self::is_builtin(global_name) || self.known_native_globals.contains(global_name);

        if is_known_native {
            // Emit CallGlobalNative directly - no runtime patching needed
            self.current
                .emit_a(OpCode::CallGlobalNative, dest, global_idx, nargs, line);

            // Emit cache word 1: initially 0 (to be populated on first call)
            self.current.push_raw(0);

            // Emit cache word 2: initially 0 (arity will be populated on first call)
            self.current.push_raw(0);
        } else {
            // Unknown global type - emit CallGlobal for runtime patching
            let slot_id = self.next_call_site_slot;
            self.next_call_site_slot += 1;

            self.current
                .emit_a(OpCode::CallGlobal, dest, global_idx, nargs, line);

            // Emit cache word 1: initially 0 (no cached ptr yet)
            self.current.push_raw(0);

            // Emit cache word 2: slot_id in lower 16 bits
            self.current.push_raw(slot_id as u32);
        }

        // Record line info for the cache words (same line as the instruction)
        self.current.record_lines(2, line);
    }
}
