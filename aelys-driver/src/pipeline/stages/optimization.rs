use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_opt::{OptimizationLevel, Optimizer};

pub struct OptimizationStage { level: OptimizationLevel } // Typed AST -> optimized Typed AST

impl OptimizationStage {
    pub fn new(level: OptimizationLevel) -> Self { Self { level } }
    pub fn level(&self) -> OptimizationLevel { self.level }
}

impl Default for OptimizationStage { fn default() -> Self { Self::new(OptimizationLevel::Standard) } }

impl Stage for OptimizationStage {
    fn name(&self) -> &str { "optimization" }

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

        let mut optimizer = Optimizer::new(self.level);
        let optimized = optimizer.optimize(typed_program);

        Ok(StageOutput::TypedAst(optimized, source))
    }
}
