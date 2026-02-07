use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_sema::TypeInference;
use std::collections::HashSet;

// AST -> Typed AST
pub struct TypeInferenceStage {
    module_aliases: HashSet<String>,
    known_globals: HashSet<String>,
}

impl TypeInferenceStage {
    pub fn new() -> Self {
        Self {
            module_aliases: HashSet::new(),
            known_globals: HashSet::new(),
        }
    }
    pub fn with_imports(module_aliases: HashSet<String>, known_globals: HashSet<String>) -> Self {
        Self {
            module_aliases,
            known_globals,
        }
    }
}

impl Default for TypeInferenceStage {
    fn default() -> Self {
        Self::new()
    }
}

impl Stage for TypeInferenceStage {
    fn name(&self) -> &str {
        "type_inference"
    }

    fn execute(&mut self, input: StageInput) -> Result<StageOutput, PipelineError> {
        let (stmts, source) = match input {
            StageInput::Ast(a, s) => (a, s),
            other => {
                return Err(PipelineError::TypeMismatch {
                    expected: "Ast",
                    got: other.type_name(),
                });
            }
        };

        let typed_program = if self.module_aliases.is_empty() && self.known_globals.is_empty() {
            TypeInference::infer_program(stmts, source.clone())
        } else {
            TypeInference::infer_program_with_imports(
                stmts,
                source.clone(),
                self.module_aliases.clone(),
                self.known_globals.clone(),
            )
        }
        .map_err(|errors| {
            let msg = errors
                .iter()
                .map(|e| format!("{}", e))
                .collect::<Vec<_>>()
                .join("\n");
            PipelineError::StageError {
                stage: "type_inference".to_string(),
                message: msg,
            }
        })?;

        Ok(StageOutput::TypedAst(typed_program, source))
    }
}
