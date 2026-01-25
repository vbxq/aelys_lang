use super::{
    ConstantFolder, DeadCodeEliminator, GlobalConstantPropagator, OptimizationLevel,
    OptimizationPass, OptimizationStats, UnusedVarEliminator,
};
use aelys_sema::TypedProgram;

// orchestrates optimization passes in the right order
pub struct Optimizer {
    level: OptimizationLevel,
    passes: Vec<Box<dyn OptimizationPass>>,
    total_stats: OptimizationStats,
}

impl Optimizer {
    pub fn new(level: OptimizationLevel) -> Self {
        let mut passes: Vec<Box<dyn OptimizationPass>> = Vec::new();

        // TODO: -O3 should do more - loop unrolling? inlining? need profiling data first
        match level {
            OptimizationLevel::None => {}
            OptimizationLevel::Basic => {
                passes.push(Box::new(ConstantFolder::new()));
            }
            OptimizationLevel::Standard | OptimizationLevel::Aggressive => {
                // const prop first so folder has more to work with
                passes.push(Box::new(GlobalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
                passes.push(Box::new(DeadCodeEliminator::new()));
                passes.push(Box::new(UnusedVarEliminator::new()));
                // second fold pass catches stuff exposed by DCE
                passes.push(Box::new(ConstantFolder::new()));
            }
        }

        Self { level, passes, total_stats: OptimizationStats::new() }
    }

    pub fn level(&self) -> OptimizationLevel { self.level }

    pub fn optimize(&mut self, mut program: TypedProgram) -> TypedProgram {
        for pass in &mut self.passes {
            self.total_stats.merge(&pass.run(&mut program));
        }
        program
    }

    pub fn stats(&self) -> &OptimizationStats { &self.total_stats }
}

impl Default for Optimizer {
    fn default() -> Self { Self::new(OptimizationLevel::Standard) }
}
