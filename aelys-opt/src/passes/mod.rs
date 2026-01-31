// optimization passes - runs on typed AST before codegen

mod constant_fold;
mod dead_code;
mod global_const_prop;
mod inline;
mod level;
mod local_const_prop;
mod optimizer;
mod pass;
mod stats;
mod unused_vars;

pub use constant_fold::ConstantFolder;
pub use dead_code::DeadCodeEliminator;
pub use global_const_prop::GlobalConstantPropagator;
pub use inline::FunctionInliner;
pub use level::OptimizationLevel;
pub use local_const_prop::LocalConstantPropagator;
pub use optimizer::Optimizer;
pub use pass::OptimizationPass;
pub use stats::OptimizationStats;
pub use unused_vars::UnusedVarEliminator;
