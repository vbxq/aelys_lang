use super::{ControlFlowGraph, LivenessAnalysis};
use aelys_sema::{TypedFunction, TypedStmt};

impl LivenessAnalysis {
    pub fn analyze_function(func: &TypedFunction) -> Self {
        let mut analysis = Self {
            live_in: std::collections::HashMap::new(),
            live_out: std::collections::HashMap::new(),
            def: std::collections::HashMap::new(),
            use_set: std::collections::HashMap::new(),
            last_use_point: std::collections::HashMap::new(),
            captured_vars: std::collections::HashSet::new(),
        };

        for (name, _) in &func.captures {
            analysis.captured_vars.insert(name.clone());
        }

        let cfg = analysis.build_cfg(&func.body);
        analysis.compute_def_use(&func.body);
        analysis.compute_liveness(&cfg);
        analysis.compute_last_use_points(&func.body);

        analysis
    }

    fn build_cfg(&self, stmts: &[TypedStmt]) -> ControlFlowGraph {
        super::cfg::build_cfg(stmts)
    }

    fn compute_def_use(&mut self, stmts: &[TypedStmt]) {
        super::def_use::compute_def_use(self, stmts)
    }

    fn compute_liveness(&mut self, cfg: &ControlFlowGraph) {
        super::liveness::compute_liveness(self, cfg)
    }

    fn compute_last_use_points(&mut self, stmts: &[TypedStmt]) {
        super::last_use::compute_last_use_points(self, stmts)
    }
}
