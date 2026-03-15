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

fn is_in_vm_range(value: i64) -> bool {
    (INT_MIN..=INT_MAX).contains(&value)
}

/// Truncate an i64 constant to the bit width of a narrower integer type so
/// that subsequent folds (comparisons, further arithmetic) operate on the
/// same bit pattern the target hardware would produce at runtime.
pub(super) fn truncate_to_type(value: i64, ty: &aelys_sema::InferType) -> i64 {
    use aelys_sema::InferType;
    match ty {
        InferType::I8 => (value as i8) as i64,
        InferType::I16 => (value as i16) as i64,
        InferType::I32 => (value as i32) as i64,
        InferType::U8 => (value as u8) as i64,
        InferType::U16 => (value as u16) as i64,
        InferType::U32 => (value as u32) as i64,
        _ => value,
    }
}

pub struct ConstantFolder {
    stats: OptimizationStats,
}

impl ConstantFolder {
    pub fn new() -> Self {
        Self {
            stats: OptimizationStats::new(),
        }
    }
}

impl Default for ConstantFolder {
    fn default() -> Self {
        Self::new()
    }
}

impl OptimizationPass for ConstantFolder {
    fn name(&self) -> &'static str {
        "constant_fold"
    }

    fn run(&mut self, program: &mut TypedProgram) -> OptimizationStats {
        self.stats = OptimizationStats::new();
        for stmt in &mut program.stmts {
            self.optimize_stmt(stmt);
        }
        self.stats.clone()
    }
}
