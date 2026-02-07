use std::collections::{HashMap, HashSet};

// FIXME: liveness analysis is pretty basic, doesn't handle all control flow

mod analysis;
mod cfg;
mod def_use;
mod last_use;
#[allow(clippy::module_inception)]
mod liveness;

#[derive(Debug, Clone, Default)]
pub struct BasicBlock {
    pub id: usize,
    pub stmts: Vec<usize>,
    pub successors: Vec<usize>,
    pub predecessors: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    pub blocks: Vec<BasicBlock>,
    pub stmt_to_block: HashMap<usize, usize>,
    pub entry: usize,
    pub exits: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct LivenessAnalysis {
    pub live_in: HashMap<usize, HashSet<String>>,
    pub live_out: HashMap<usize, HashSet<String>>,
    pub def: HashMap<usize, HashSet<String>>,
    pub use_set: HashMap<usize, HashSet<String>>,
    pub last_use_point: HashMap<String, usize>,
    pub captured_vars: HashSet<String>,
}

impl LivenessAnalysis {
    pub fn is_dead_after(&self, var_name: &str, stmt_idx: usize) -> bool {
        if self.captured_vars.contains(var_name) {
            return false;
        }

        if let Some(&last_use) = self.last_use_point.get(var_name) {
            return stmt_idx >= last_use;
        }

        true
    }

    pub fn get_dead_vars_after(
        &self,
        stmt_idx: usize,
        defined_vars: &HashSet<String>,
    ) -> Vec<String> {
        defined_vars
            .iter()
            .filter(|var| self.is_dead_after(var, stmt_idx))
            .cloned()
            .collect()
    }
}
