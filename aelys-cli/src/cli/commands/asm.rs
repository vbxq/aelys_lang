// disassembly for debugging bytecode

use crate::cli::vm_config::parse_vm_args_or_error;
use aelys_backend::Compiler;
use aelys_bytecode::asm::{deserialize_with_manifest, disassemble_to_string};
use aelys_driver::modules::load_modules_with_loader;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::{Source, StmtKind};
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub fn asm_transform(path: &Path) -> Result<PathBuf, String> {
    match asm_transform_with_options(
        path,
        None,
        false,
        OptimizationLevel::Standard,
        VmConfig::default(),
    )? {
        Some(path) => Ok(path),
        None => Err("no output produced".to_string()),
    }
}

pub fn run_with_options(
    path: &str,
    output: Option<String>,
    stdout: bool,
    opt_level: OptimizationLevel,
    vm_args: Vec<String>,
) -> Result<i32, String> {
    let parsed = parse_vm_args_or_error(&vm_args)?;
    let config = parsed.config;

    let output_path = output.map(PathBuf::from);
    let output =
        asm_transform_with_options(Path::new(path), output_path, stdout, opt_level, config)?;
    if let Some(path) = output {
        eprintln!("Wrote {}", path.display());
    }
    Ok(0)
}

fn asm_transform_with_options(
    path: &Path,
    output: Option<PathBuf>,
    stdout: bool,
    opt_level: OptimizationLevel,
    config: VmConfig,
) -> Result<Option<PathBuf>, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "aelys" => disassemble_source(path, output, stdout, opt_level, config),
        "avbc" => disassemble_avbc(path, output, stdout),
        "aasm" => Err("input is already assembly".to_string()),
        _ => Err(format!(
            "unsupported input extension '{}', expected .aelys or .avbc",
            ext
        )),
    }
}

fn disassemble_source(
    path: &Path,
    output: Option<PathBuf>,
    stdout: bool,
    opt_level: OptimizationLevel,
    config: VmConfig,
) -> Result<Option<PathBuf>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let (function, heap) = compile_source(path, &content, opt_level, config)?;
    let text = disassemble_to_string(&function, Some(&heap));
    write_output(path, output, stdout, text)
}

fn disassemble_avbc(
    path: &Path,
    output: Option<PathBuf>,
    stdout: bool,
) -> Result<Option<PathBuf>, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let (function, heap, _manifest, _bundles) =
        deserialize_with_manifest(&bytes).map_err(|err| err.to_string())?;
    let text = disassemble_to_string(&function, Some(&heap));
    write_output(path, output, stdout, text)
}

fn write_output(
    input: &Path,
    output: Option<PathBuf>,
    stdout: bool,
    text: String,
) -> Result<Option<PathBuf>, String> {
    if stdout {
        println!("{}", text);
        return Ok(None);
    }
    let output = output.unwrap_or_else(|| output_path_for(input, "aasm"));
    std::fs::write(&output, text)
        .map_err(|err| format!("failed to write {}: {}", output.display(), err))?;
    Ok(Some(output))
}

fn compile_source(
    path: &Path,
    content: &str,
    opt_level: OptimizationLevel,
    config: VmConfig,
) -> Result<(aelys_bytecode::Function, aelys_bytecode::Heap), String> {
    let name = path.display().to_string();
    let src = Source::new(&name, content);

    let tokens = Lexer::with_source(src.clone())
        .scan()
        .map_err(|err| err.to_string())?;
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .map_err(|err| err.to_string())?;

    let mut vm =
        VM::with_config_and_args(src.clone(), config, Vec::new()).map_err(|err| err.to_string())?;

    let (imports, _loader) = load_modules_with_loader(&stmts, path, src.clone(), &mut vm)
        .map_err(|err| err.to_string())?;

    let main_stmts: Vec<_> = stmts
        .into_iter()
        .filter(|stmt| !matches!(stmt.kind, StmtKind::Needs(_)))
        .collect();

    let mut all_known_globals = imports.known_globals.clone();
    for builtin in ["alloc", "free", "load", "store", "type"] {
        all_known_globals.insert(builtin.to_string());
    }

    let typed_program = aelys_sema::TypeInference::infer_program_with_imports(
        main_stmts,
        src.clone(),
        imports.module_aliases.clone(),
        all_known_globals,
    )
    .map_err(|errors| {
        if let Some(err) = errors.first() {
            err.to_string()
        } else {
            "Unknown type error".to_string()
        }
    })?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(typed_program);

    let (function, heap, _globals) = Compiler::with_modules(
        None,
        src.clone(),
        imports.module_aliases,
        imports.known_globals,
        imports.known_native_globals,
        imports.symbol_origins,
    )
    .compile_typed(&typed_program)
    .map_err(|err| err.to_string())?;

    Ok((function, heap))
}

fn output_path_for(path: &Path, extension: &str) -> PathBuf {
    let mut output = path.to_path_buf();
    output.set_extension(extension);
    output
}
