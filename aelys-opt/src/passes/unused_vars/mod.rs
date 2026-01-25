// unused variable elimination - removes `let x = ...` if x is never read
// won't remove if initializer has side effects (function calls)

mod analysis;
mod eliminate;

use super::{OptimizationPass, OptimizationStats};
use aelys_sema::TypedProgram;
use std::collections::HashSet;

pub struct UnusedVarEliminator {
    stats: OptimizationStats,
}

impl UnusedVarEliminator {
    pub fn new() -> Self { Self { stats: OptimizationStats::new() } }
}

impl Default for UnusedVarEliminator {
    fn default() -> Self { Self::new() }
}

impl OptimizationPass for UnusedVarEliminator {
    fn name(&self) -> &'static str { "unused_var_elimination" }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.stats = OptimizationStats::new();
        let used_vars = analysis::collect_used_vars(&program.stmts);
        self.eliminate_unused(&mut program.stmts, &used_vars);
        self.stats.clone()
    }
}

impl UnusedVarEliminator {
    fn eliminate_unused(&mut self, stmts: &mut Vec<aelys_sema::TypedStmt>, used_vars: &HashSet<String>) {
        eliminate::eliminate_unused_in_block(stmts, used_vars, &mut self.stats);
    }
}
