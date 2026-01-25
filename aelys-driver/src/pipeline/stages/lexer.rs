use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_frontend::lexer::Lexer;

pub struct LexerStage; // Source -> Tokens

impl Stage for LexerStage {
    fn name(&self) -> &str { "lexer" }

    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError> {
        let source = match input {
            StageInput::Source(s) => s,
            other => {
                return Err(PipelineError::TypeMismatch {
                    expected: "Source",
                    got: other.type_name(),
                });
            }
        };

        let tokens =
            Lexer::with_source(source.clone())
                .scan()
                .map_err(|e| PipelineError::StageError {
                    stage: "lexer".to_string(),
                    message: e.to_string(),
                })?;

        Ok(StageOutput::Tokens(tokens, source))
    }
}
