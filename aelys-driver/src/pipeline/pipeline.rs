use super::cache::{CachedOutput, source_hash};
use super::types::{PipelineError, Stage, StageInput, StageOutput};
use aelys_bytecode::{Function, Heap};
use aelys_runtime::Value;
use aelys_syntax::Source;
use std::collections::HashMap;
use std::sync::Arc;

// TODO: LRU cache eviction? unbounded cache could get big on long REPL sessions

// caches intermediate results by source hash
pub struct Pipeline {
    pub(crate) stages: Vec<Box<dyn Stage>>,
    pub(crate) cache: HashMap<(String, u64), CachedOutput>,
}

impl Pipeline {
    pub fn new() -> Self { Self { stages: Vec::new(), cache: HashMap::new() } }
    pub fn add_stage(&mut self, stage: Box<dyn Stage>) { self.stages.push(stage); }
    pub fn clear_cache(&mut self) { self.cache.clear(); }
    pub fn cache_size(&self) -> usize { self.cache.len() }

    pub(crate) fn exec(&mut self, source: Arc<Source>) -> Result<Value, PipelineError> {
        let hash = source_hash(&source);
        let mut current = StageInput::Source(source);

        for stage in &mut self.stages {
            let stage_name = stage.name().to_string();
            let cache_key = (stage_name.clone(), hash);

            if stage.cacheable() {
                if let Some(cached) = self.cache.get(&cache_key) {
                    if cached.source_hash == hash {
                        let output = cached.output.clone();
                        if let StageOutput::Value(v) = output {
                            return Ok(v);
                        }
                        current = output.into_input()?;
                        continue;
                    }
                }
            }

            let output = stage.execute(current)?;

            if stage.cacheable() {
                self.cache.insert(
                    cache_key,
                    CachedOutput {
                        source_hash: hash,
                        output: output.clone(),
                    },
                );
            }

            if let StageOutput::Value(v) = output {
                return Ok(v);
            }

            current = output.into_input()?;
        }

        Err(PipelineError::MissingInput {
            stage: "final".to_string(),
        })
    }

    pub(crate) fn compile_internal(
        &mut self,
        input: StageInput,
    ) -> Result<(Function, Heap), PipelineError> {
        let hash = match &input {
            StageInput::Source(source)
            | StageInput::Tokens(_, source)
            | StageInput::Ast(_, source)
            | StageInput::TypedAst(_, source)
            | StageInput::Compiled(_, _, source) => source_hash(source),
        };

        let mut current = input;

        for stage in &mut self.stages {
            let stage_name = stage.name().to_string();

            if stage_name == "vm" {
                break;
            }

            let cache_key = (stage_name.clone(), hash);

            if stage.cacheable() {
                if let Some(cached) = self.cache.get(&cache_key) {
                    if cached.source_hash == hash {
                        current = cached.output.clone().into_input()?;
                        continue;
                    }
                }
            }

            let output = stage.execute(current)?;

            if stage.cacheable() {
                self.cache.insert(
                    cache_key,
                    CachedOutput {
                        source_hash: hash,
                        output: output.clone(),
                    },
                );
            }

            current = output.into_input()?;
        }

        match current {
            StageInput::Compiled(func, heap, _) => Ok((func, heap)),
            other => Err(PipelineError::TypeMismatch {
                expected: "Compiled",
                got: other.type_name(),
            }),
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}
