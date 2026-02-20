use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};

pub struct AirLowerStage;

impl Stage for AirLowerStage {
    fn name(&self) -> &str {
        "air_lower"
    }

    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError> {
        let (typed_program, source) = match input {
            StageInput::TypedAst(t, s) => (t, s),
            other => {
                return Err(PipelineError::TypeMismatch {
                    expected: "TypedAst",
                    got: other.type_name(),
                });
            }
        };

        let _air_program = aelys_air::lower::lower(&typed_program);

        // TODO! Here !
        // we return typedast FOR NOW
        Ok(StageOutput::TypedAst(typed_program, source))
    }
}
