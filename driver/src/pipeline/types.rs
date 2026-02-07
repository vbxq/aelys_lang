use aelys_bytecode::{Function, Heap};
use aelys_runtime::Value;
use aelys_sema::TypedProgram;
use aelys_syntax::{Source, Stmt, Token};
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub enum PipelineError {
    StageError {
        stage: String,
        message: String,
    },
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
    },
    MissingInput {
        stage: String,
    },
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineError::StageError { stage, message } => {
                write!(f, "Stage '{}' failed: {}", stage, message)
            }
            PipelineError::TypeMismatch { expected, got } => {
                write!(f, "Type mismatch: expected {}, got {}", expected, got)
            }
            PipelineError::MissingInput { stage } => {
                write!(f, "Missing input for stage '{}'", stage)
            }
        }
    }
}

impl std::error::Error for PipelineError {}

// not Send - VM has raw pointers, keep pipelines single-threaded
pub trait Stage {
    fn name(&self) -> &str;
    fn cacheable(&self) -> bool {
        true
    }
    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError>;
}

#[derive(Clone)]
pub enum StageInput {
    Source(Arc<Source>),
    Tokens(Vec<Token>, Arc<Source>),
    Ast(Vec<Stmt>, Arc<Source>),
    TypedAst(TypedProgram, Arc<Source>),
    Compiled(Box<Function>, Heap, Arc<Source>),
}

impl StageInput {
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            StageInput::Source(_) => "Source",
            StageInput::Tokens(_, _) => "Tokens",
            StageInput::Ast(_, _) => "Ast",
            StageInput::TypedAst(_, _) => "TypedAst",
            StageInput::Compiled(_, _, _) => "Compiled",
        }
    }
}

#[derive(Clone)]
pub enum StageOutput {
    Tokens(Vec<Token>, Arc<Source>),
    Ast(Vec<Stmt>, Arc<Source>),
    TypedAst(TypedProgram, Arc<Source>),
    Compiled(Box<Function>, Heap, Arc<Source>),
    Value(Value),
}

impl StageOutput {
    pub(crate) fn into_input(self) -> Result<StageInput, PipelineError> {
        match self {
            StageOutput::Tokens(t, s) => Ok(StageInput::Tokens(t, s)),
            StageOutput::Ast(a, s) => Ok(StageInput::Ast(a, s)),
            StageOutput::TypedAst(t, s) => Ok(StageInput::TypedAst(t, s)),
            StageOutput::Compiled(f, h, s) => Ok(StageInput::Compiled(f, h, s)),

            StageOutput::Value(_) => Err(PipelineError::TypeMismatch {
                expected: "non-final output",
                got: "Value",
            }),
        }
    }
}
