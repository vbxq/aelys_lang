use super::{ControlFlowGraph, LivenessAnalysis};
use std::collections::HashSet;

pub(super) fn compute_liveness(analysis: &mut LivenessAnalysis, cfg: &ControlFlowGraph) {
    for block in &cfg.blocks {
        analysis.live_in.insert(block.id, HashSet::new());
        analysis.live_out.insert(block.id, HashSet::new());
    }

    let mut changed = true;
    while changed {
        changed = false;

        for block_id in (0..cfg.blocks.len()).rev() {
            let block = &cfg.blocks[block_id];

            let mut new_out: HashSet<String> = HashSet::new();
            for &succ_id in &block.successors {
                if let Some(succ_in) = analysis.live_in.get(&succ_id) {
                    new_out.extend(succ_in.iter().cloned());
                }
            }

            let old_out = analysis
                .live_out
                .get(&block_id)
                .cloned()
                .unwrap_or_default();
            if new_out != old_out {
                changed = true;
                analysis.live_out.insert(block_id, new_out.clone());
            }

            let mut new_in = new_out.clone();

            for &stmt_idx in block.stmts.iter().rev() {
                if let Some(defs) = analysis.def.get(&stmt_idx) {
                    for def in defs {
                        new_in.remove(def);
                    }
                }
                if let Some(uses) = analysis.use_set.get(&stmt_idx) {
                    new_in.extend(uses.iter().cloned());
                }
            }

            let old_in = analysis.live_in.get(&block_id).cloned().unwrap_or_default();
            if new_in != old_in {
                changed = true;
                analysis.live_in.insert(block_id, new_in);
            }
        }
    }
}
