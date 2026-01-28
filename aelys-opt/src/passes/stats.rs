use std::fmt;

#[derive(Debug, Clone, Default)]
pub struct OptimizationStats {
    pub globals_propagated: usize,
    pub locals_propagated: usize,
    pub constants_folded: usize,
    pub dead_code_eliminated: usize,
    pub branches_eliminated: usize,
}

impl OptimizationStats {
    pub fn new() -> Self { Self::default() }

    pub fn merge(&mut self, other: &OptimizationStats) {
        self.globals_propagated += other.globals_propagated;
        self.locals_propagated += other.locals_propagated;
        self.constants_folded += other.constants_folded;
        self.dead_code_eliminated += other.dead_code_eliminated;
        self.branches_eliminated += other.branches_eliminated;
    }

    pub fn has_changes(&self) -> bool {
        self.globals_propagated + self.locals_propagated + self.constants_folded +
        self.dead_code_eliminated + self.branches_eliminated > 0
    }
}

impl fmt::Display for OptimizationStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "globals: {}, locals: {}, folded: {}, dce: {}, branches: {}",
            self.globals_propagated, self.locals_propagated, self.constants_folded,
            self.dead_code_eliminated, self.branches_eliminated)
    }
}
