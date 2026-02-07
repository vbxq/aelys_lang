use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_frontend::parser::Parser;

pub struct ParserStage; // Tokens -> AST

impl Stage for ParserStage {
    fn name(&self) -> &str {
        "parser"
    }

    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError> {
        let (tokens, source) = match input {
            StageInput::Tokens(t, s) => (t, s),
            other => {
                return Err(PipelineError::TypeMismatch {
                    expected: "Tokens",
                    got: other.type_name(),
                });
            }
        };

        let stmts =
            Parser::new(tokens, source.clone())
                .parse()
                .map_err(|e| PipelineError::StageError {
                    stage: "parser".to_string(),
                    message: e.to_string(),
                })?;

        Ok(StageOutput::Ast(stmts, source))
    }
}
