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

        let mut air_program = aelys_air::lower::lower(&typed_program);
        aelys_air::layout::compute_layouts(&mut air_program);
        let air_program = aelys_air::mono::monomorphize(air_program);

        Ok(StageOutput::Air(air_program, typed_program, source))
    }
}
