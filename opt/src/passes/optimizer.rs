use super::{
    ConstantFolder, DeadCodeEliminator, FunctionInliner, GlobalConstantPropagator,
    LocalConstantPropagator, OptimizationLevel, OptimizationPass, OptimizationStats,
    UnusedVarEliminator,
};
use aelys_air::bir::Checked;
use aelys_common::Warning;
use aelys_sema::{TypedProgram, TypedStmt, TypedStmtKind};

pub struct Optimizer {
    level: OptimizationLevel,
    inliner: FunctionInliner,
    passes: Vec<Box<dyn OptimizationPass>>,
    total_stats: OptimizationStats,
    collected_warnings: Vec<Warning>,
}

impl Optimizer {
    pub fn new(level: OptimizationLevel) -> Self {
        let mut passes: Vec<Box<dyn OptimizationPass>> = Vec::new();
        let inliner = FunctionInliner::new(level);

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

    /// optimizer without first running the borrow/move check is a compile error, not a convention.
    pub fn optimize(&mut self, checked: Checked) -> TypedProgram {
        let mut program = checked.into_inner();
        self.collected_warnings.clear();

        let frozen: Vec<(usize, TypedStmt)> = program
            .stmts
            .iter()
            .enumerate()
            .filter(|(_, stmt)| matches!(stmt.kind, TypedStmtKind::Let { .. }))
            .map(|(at, stmt)| (at, stmt.clone()))
            .collect();

        self.total_stats.merge(&self.inliner.run(&mut program));
        self.collected_warnings.extend(self.inliner.take_warnings());

        for pass in &mut self.passes {
            self.total_stats.merge(&pass.run(&mut program));
        }

        program
            .stmts
            .retain(|stmt| !matches!(stmt.kind, TypedStmtKind::Let { .. }));
        for (at, stmt) in frozen {
            let at = at.min(program.stmts.len());
            program.stmts.insert(at, stmt);
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
