use aelys_backend::Compiler;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, Value, VmConfig};
use aelys_sema::TypeInference;
use aelys_syntax::{Source, Span};

pub fn run(source: &str, name: &str) -> Result<Value> {
    run_with_config(source, name, VmConfig::default(), Vec::new())
}

pub fn run_with_config(
    source: &str,
    name: &str,
    config: VmConfig,
    program_args: Vec<String>,
) -> Result<Value> {
    run_with_config_and_opt(
        source,
        name,
        config,
        program_args,
        OptimizationLevel::Standard,
    )
}

pub fn run_with_config_and_opt(
    source: &str,
    name: &str,
    config: VmConfig,
    program_args: Vec<String>,
    opt_level: OptimizationLevel,
) -> Result<Value> {
    let src = Source::new(name, source);
    let tokens = Lexer::with_source(src.clone()).scan()?;
    let stmts = Parser::new(tokens, src.clone()).parse()?;

    let typed_program = TypeInference::infer_program(stmts, src.clone()).map_err(|errors| {
        if let Some(err) = errors.first() {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::TypeInferenceError(format!("{}", err)),
                err.span,
                src.clone(),
            ))
        } else {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::TypeInferenceError("Unknown type error".to_string()),
                Span::dummy(),
                src.clone(),
            ))
        }
    })?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(typed_program);

    let (mut function, mut compile_heap, _globals) =
        Compiler::new(None, src.clone()).compile_typed(&typed_program)?;

    let mut vm =
        VM::with_config_and_args(src, config, program_args).map_err(AelysError::Runtime)?;
    let remap = vm
        .merge_heap(&mut compile_heap)
        .map_err(AelysError::Runtime)?;
    function.remap_constants(&remap);

    let func_ref = vm.alloc_function(function).map_err(AelysError::Runtime)?;
    Ok(vm.execute(func_ref)?)
}

// convenience for tests/embedding
pub fn run_source(source: &str, name: &str, opt_level: Option<OptimizationLevel>) -> Result<Value> {
    run_with_config_and_opt(
        source,
        name,
        VmConfig::default(),
        Vec::new(),
        opt_level.unwrap_or(OptimizationLevel::Standard),
    )
}
