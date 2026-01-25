// pipeline orchestration - chains lexer->parser->sema->opt->codegen->vm

mod cache;
mod compile;
mod pipeline;
mod run;
mod standard;
mod types;

pub mod stages;

pub use pipeline::Pipeline;
pub use stages::{
    CompilerStage, LexerStage, OptimizationStage, ParserStage, TypeInferenceStage, VMStage,
};
pub use standard::{
    compilation_pipeline, compilation_pipeline_with_modules, compilation_pipeline_with_opt,
    standard_pipeline, standard_pipeline_with_opt,
};
pub use types::{PipelineError, Stage, StageInput, StageOutput};
