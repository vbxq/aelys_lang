use crate::pipeline::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_backend::Compiler;
use std::collections::{HashMap, HashSet};

// Typed AST -> Compiled bytecode
pub struct CompilerStage {
    module_aliases: HashSet<String>,
    known_globals: HashSet<String>,
    known_native_globals: HashSet<String>,
    symbol_origins: HashMap<String, String>,
}

impl CompilerStage {
    pub fn new() -> Self {
        Self {
            module_aliases: HashSet::new(),
            known_globals: HashSet::new(),
            known_native_globals: HashSet::new(),
            symbol_origins: HashMap::new(),
        }
    }

    pub fn with_modules(
        module_aliases: HashSet<String>,
        known_globals: HashSet<String>,
        known_native_globals: HashSet<String>,
        symbol_origins: HashMap<String, String>,
    ) -> Self {
        Self { module_aliases, known_globals, known_native_globals, symbol_origins }
    }
}

impl Default for CompilerStage { fn default() -> Self { Self::new() } }

impl Stage for CompilerStage {
    fn name(&self) -> &str { "compiler" }

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

        let (function, heap, _globals) =
            if self.module_aliases.is_empty() && self.known_globals.is_empty() {
                Compiler::new(None, source.clone()).compile_typed(&typed_program)
            } else {
                Compiler::with_modules(
                    None,
                    source.clone(),
                    self.module_aliases.clone(),
                    self.known_globals.clone(),
                    self.known_native_globals.clone(),
                    self.symbol_origins.clone(),
                )
                .compile_typed(&typed_program)
            }
            .map_err(|e| PipelineError::StageError {
                stage: "compiler".to_string(),
                message: e.to_string(),
            })?;

        Ok(StageOutput::Compiled(function, heap, source))
    }
}
