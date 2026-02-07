// global constant propagation - inlines immutable top-level let bindings
// works in passes to handle `let A = 1; let B = A + 1;` chains

mod collect;
mod substitute;

use super::{OptimizationPass, OptimizationStats};
use aelys_sema::TypedProgram;
use std::collections::HashMap;

pub struct GlobalConstantPropagator {
    constants: HashMap<String, aelys_sema::TypedExpr>,
    stats: OptimizationStats,
}

impl GlobalConstantPropagator {
    pub fn new() -> Self {
        Self {
            constants: HashMap::new(),
            stats: OptimizationStats::new(),
        }
    }
}

impl Default for GlobalConstantPropagator {
    fn default() -> Self {
        Self::new()
    }
}

impl OptimizationPass for GlobalConstantPropagator {
    fn name(&self) -> &'static str {
        "global_const_prop"
    }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.constants.clear();
        self.stats = OptimizationStats::new();

        self.collect_global_constants(&program.stmts);
        for stmt in &mut program.stmts {
            self.substitute_in_stmt(stmt);
        }

        self.stats.clone()
    }
}
