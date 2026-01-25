// constant folding - evaluates compile-time constant expressions
// handles: arithmetic, comparisons, string concat, boolean logic
// TODO: fold pure builtins like len("hello") -> 5

mod expr;
mod stmt;

use super::{OptimizationPass, OptimizationStats};
use aelys_sema::TypedProgram;

const MAX_FOLDED_STRING_LEN: usize = 4096; // don't bloat constant pool with huge strings

// 48-bit signed range (NaN-boxing payload limit)
const INT_MIN: i64 = -(1i64 << 47);
const INT_MAX: i64 = (1i64 << 47) - 1;

fn is_in_vm_range(value: i64) -> bool { value >= INT_MIN && value <= INT_MAX }

pub struct ConstantFolder {
    stats: OptimizationStats,
}

impl ConstantFolder {
    pub fn new() -> Self { Self { stats: OptimizationStats::new() } }
}

impl Default for ConstantFolder {
    fn default() -> Self { Self::new() }
}

impl OptimizationPass for ConstantFolder {
    fn name(&self) -> &'static str { "constant_fold" }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.stats = OptimizationStats::new();
        for stmt in &mut program.stmts { self.optimize_stmt(stmt); }
        self.stats.clone()
    }
}
