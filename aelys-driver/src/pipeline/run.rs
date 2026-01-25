use super::pipeline::Pipeline;
use super::types::PipelineError;
use aelys_runtime::Value;
use aelys_syntax::Source;
use std::sync::Arc;

impl Pipeline {
    pub fn execute(&mut self, source: Arc<Source>) -> Result<Value, PipelineError> { self.exec(source) }
    pub fn execute_str(&mut self, name: &str, source: &str) -> Result<Value, PipelineError> { self.execute(Source::new(name, source)) }
}
