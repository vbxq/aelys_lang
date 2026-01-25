use super::pipeline::Pipeline;
use super::stages::{
    CompilerStage, LexerStage, OptimizationStage, ParserStage, TypeInferenceStage, VMStage,
};
use aelys_opt::OptimizationLevel;
use std::collections::HashSet;

pub fn standard_pipeline() -> Pipeline { standard_pipeline_with_opt(OptimizationLevel::Standard) }

pub fn standard_pipeline_with_opt(opt_level: OptimizationLevel) -> Pipeline {
    let mut pipeline = Pipeline::new();
    pipeline.add_stage(Box::new(LexerStage));
    pipeline.add_stage(Box::new(ParserStage));
    pipeline.add_stage(Box::new(TypeInferenceStage::new()));
    pipeline.add_stage(Box::new(OptimizationStage::new(opt_level)));
    pipeline.add_stage(Box::new(CompilerStage::new()));
    pipeline.add_stage(Box::new(VMStage::new()));
    pipeline
}

// no VM - use pipeline.compile() to get (Function, Heap)
pub fn compilation_pipeline() -> Pipeline { compilation_pipeline_with_opt(OptimizationLevel::Standard) }

pub fn compilation_pipeline_with_opt(opt_level: OptimizationLevel) -> Pipeline {
    let mut pipeline = Pipeline::new();
    pipeline.add_stage(Box::new(LexerStage));
    pipeline.add_stage(Box::new(ParserStage));
    pipeline.add_stage(Box::new(TypeInferenceStage::new()));
    pipeline.add_stage(Box::new(OptimizationStage::new(opt_level)));
    pipeline.add_stage(Box::new(CompilerStage::new()));
    pipeline
}

// for programs with `needs` statements
pub fn compilation_pipeline_with_modules(
    opt_level: OptimizationLevel,
    module_aliases: HashSet<String>,
    known_globals: HashSet<String>,
    known_native_globals: HashSet<String>,
) -> Pipeline {
    let mut pipeline = Pipeline::new();
    pipeline.add_stage(Box::new(LexerStage));
    pipeline.add_stage(Box::new(ParserStage));
    pipeline.add_stage(Box::new(TypeInferenceStage::with_imports(
        module_aliases.clone(),
        known_globals.clone(),
    )));
    pipeline.add_stage(Box::new(OptimizationStage::new(opt_level)));
    pipeline.add_stage(Box::new(CompilerStage::with_modules(
        module_aliases,
        known_globals,
        known_native_globals,
    )));
    pipeline
}
