use super::super::Compiler;
use std::collections::HashSet;

impl Compiler {
    pub fn free_dead_locals(
        &mut self,
        stmt_idx: usize,
        liveness: &super::super::liveness::LivenessAnalysis,
        already_freed: &mut HashSet<String>,
    ) -> usize {
        let mut freed = 0;
        for local in &mut self.locals {
            if local.is_freed || already_freed.contains(&local.name) {
                continue;
            }

            if local.is_captured {
                continue;
            }

            if liveness.is_dead_after(&local.name, stmt_idx) {
                self.register_pool[local.register as usize] = false;
                local.is_freed = true;
                already_freed.insert(local.name.clone());
                freed += 1;
            }
        }
        freed
    }
}
