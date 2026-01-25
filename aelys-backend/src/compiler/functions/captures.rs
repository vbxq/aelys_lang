use super::super::{Compiler, Upvalue};

impl Compiler {
    pub fn mark_captures_from_nested(&mut self, nested: &Compiler) {
        for upvalue in &nested.upvalues {
            if upvalue.is_local {
                self.mark_local_captured(upvalue.index);
            } else if upvalue.index & 0x80 != 0 {
                let _resolved = self.resolve_upvalue(&upvalue.name);
            }
        }
    }

    pub fn fix_transitive_captures(&self, nested_upvalues: &mut [Upvalue]) {
        for upvalue in nested_upvalues.iter_mut() {
            if !upvalue.is_local && upvalue.index & 0x80 != 0 {
                if let Some((idx, _)) = self
                    .upvalues
                    .iter()
                    .enumerate()
                    .find(|(_, u)| u.name == upvalue.name)
                {
                    upvalue.index = idx as u8;
                }
            }
        }
    }
}
