use super::Function;

impl Function {
    /// Patch a jump instruction at the given offset
    pub fn patch_jump(&mut self, offset: usize) {
        let jump_dist = (self.bytecode_builder.len() - offset - 1) as i16;
        let instr = self.bytecode_builder[offset];
        let op = instr >> 24;
        let a = (instr >> 16) & 0xFF;
        self.bytecode_builder[offset] = (op << 24) | (a << 16) | ((jump_dist as u16) as u32);
    }
}
