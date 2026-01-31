use super::{
    ConstantFolder, DeadCodeEliminator, FunctionInliner, GlobalConstantPropagator,
    LocalConstantPropagator, OptimizationLevel, OptimizationPass, OptimizationStats,
    UnusedVarEliminator,
};
use aelys_common::Warning;
use aelys_sema::TypedProgram;

pub struct Optimizer {
    level: OptimizationLevel,
    inliner: Option<FunctionInliner>,
    passes: Vec<Box<dyn OptimizationPass>>,
    total_stats: OptimizationStats,
    collected_warnings: Vec<Warning>,
}

impl Optimizer {
    pub fn new(level: OptimizationLevel) -> Self {
        let mut passes: Vec<Box<dyn OptimizationPass>> = Vec::new();
        let inliner = if level != OptimizationLevel::None {
            Some(FunctionInliner::new(level))
        } else {
            None
        };

        match level {
            OptimizationLevel::None => {}

            OptimizationLevel::Basic => {
                passes.push(Box::new(LocalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
            }

            OptimizationLevel::Standard => {
                passes.push(Box::new(GlobalConstantPropagator::new()));
                passes.push(Box::new(LocalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
                passes.push(Box::new(DeadCodeEliminator::new()));
                passes.push(Box::new(UnusedVarEliminator::new()));
                passes.push(Box::new(LocalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
            }

            OptimizationLevel::Aggressive => {
                passes.push(Box::new(GlobalConstantPropagator::new()));
                passes.push(Box::new(LocalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
                passes.push(Box::new(DeadCodeEliminator::new()));
                passes.push(Box::new(UnusedVarEliminator::new()));
                passes.push(Box::new(LocalConstantPropagator::new()));
                passes.push(Box::new(ConstantFolder::new()));
                passes.push(Box::new(DeadCodeEliminator::new()));
            }
        }

        Self {
            level,
            inliner,
            passes,
            total_stats: OptimizationStats::new(),
            collected_warnings: Vec::new(),
        }
    }

    pub fn level(&self) -> OptimizationLevel {
        self.level
    }

    pub fn optimize(&mut self, mut program: TypedProgram) -> TypedProgram {
        self.collected_warnings.clear();

        if let Some(inliner) = &mut self.inliner {
            self.total_stats.merge(&inliner.run(&mut program));
            self.collected_warnings.extend(inliner.take_warnings());
        }

        for pass in &mut self.passes {
            self.total_stats.merge(&pass.run(&mut program));
        }

        program
    }

    pub fn stats(&self) -> &OptimizationStats {
        &self.total_stats
    }

    pub fn warnings(&self) -> &[Warning] {
        &self.collected_warnings
    }

    pub fn take_warnings(&mut self) -> Vec<Warning> {
        std::mem::take(&mut self.collected_warnings)
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new(OptimizationLevel::Standard)
    }
}
