// dead code elimination - removes unreachable code after return/break/continue
// TODO: interprocedural DCE (remove unused private functions)

mod expr;
mod stmt;

use super::{OptimizationPass, OptimizationStats};
use aelys_sema::TypedProgram;

pub struct DeadCodeEliminator {
    stats: OptimizationStats,
}

impl DeadCodeEliminator {
    pub fn new() -> Self { Self { stats: OptimizationStats::new() } }
}

impl Default for DeadCodeEliminator {
    fn default() -> Self { Self::new() }
}

impl OptimizationPass for DeadCodeEliminator {
    fn name(&self) -> &'static str { "dead_code_elimination" }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.stats = OptimizationStats::new();
        self.eliminate_in_block(&mut program.stmts);
        self.stats.clone()
    }
}
