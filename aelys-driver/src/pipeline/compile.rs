use super::pipeline::Pipeline;
use super::types::{PipelineError, StageInput};
use aelys_bytecode::{Function, Heap};
use aelys_syntax::{Source, Stmt};
use std::sync::Arc;

impl Pipeline {
    pub fn compile(&mut self, source: Arc<Source>) -> Result<(Function, Heap), PipelineError> {
        self.compile_internal(StageInput::Source(source))
    }

    pub fn compile_str(&mut self, name: &str, source: &str) -> Result<(Function, Heap), PipelineError> {
        self.compile(Source::new(name, source))
    }

    // skip lexer/parser - for when AST already parsed (module loading)
    pub fn compile_ast(&mut self, stmts: Vec<Stmt>, source: Arc<Source>) -> Result<(Function, Heap), PipelineError> {
        self.compile_internal(StageInput::Ast(stmts, source))
    }
}
