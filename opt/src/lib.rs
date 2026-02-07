// typed AST optimization passes

pub mod passes;

pub use passes::{
    ConstantFolder, DeadCodeEliminator, GlobalConstantPropagator, OptimizationLevel,
    OptimizationPass, OptimizationStats, Optimizer,
};
