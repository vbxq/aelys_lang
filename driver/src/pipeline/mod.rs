// pipeline orchestration - chains lexer->parser->sema->opt->codegen

#[allow(clippy::module_inception)]
mod pipeline;
mod standard;
mod types;

pub mod stages;

pub use pipeline::Pipeline;
pub use stages::{
    CompilerStage, LexerStage, OptimizationStage, ParserStage, TypeInferenceStage,
};
pub use standard::{
    compilation_pipeline, compilation_pipeline_with_modules, compilation_pipeline_with_opt,
    standard_pipeline, standard_pipeline_with_opt,
};
pub use types::{PipelineError, Stage, StageInput, StageOutput};
