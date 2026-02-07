use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_opt::OptimizationLevel;

pub struct DebugStripStage {
    level: OptimizationLevel,
}

impl DebugStripStage {
    pub fn new(level: OptimizationLevel) -> Self {
        Self { level }
    }
}

impl Stage for DebugStripStage {
    fn name(&self) -> &str {
        "debug_strip"
    }

    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError> {
        let (mut function, heap, source) = match input {
            StageInput::Compiled(f, h, s) => (*f, h, s),
            other => {
                return Err(PipelineError::TypeMismatch {
                    expected: "Compiled",
                    got: other.type_name(),
                });
            }
        };

        // Only strip debug info in optimized builds (not -O0)
        if self.level != OptimizationLevel::None {
            function.strip_debug_info();
        }

        Ok(StageOutput::Compiled(Box::new(function), heap, source))
    }
}
