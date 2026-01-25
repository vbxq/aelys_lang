// source -> avbc compiler
// TODO: incremental compilation for large projects

use aelys_backend::Compiler;
use aelys_bytecode::asm::NativeBundle;
use aelys_driver::modules::{LoadedNativeInfo, load_modules_with_loader};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_modules::manifest::Manifest;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::{Source, StmtKind};
use std::path::{Path, PathBuf};

const BUILTIN_NAMES: &[&str] = &["alloc", "free", "load", "store", "type"];

#[allow(dead_code)]
pub fn compile_to_avbc(path: &Path, opt_level: OptimizationLevel) -> Result<PathBuf, String> {
    compile_to_avbc_with_output(path, None, opt_level)
}

pub fn compile_to_avbc_with_output(
    path: &Path,
    output: Option<PathBuf>,
    opt_level: OptimizationLevel,
) -> Result<PathBuf, String> {
    match detect_format(path) {
        CompileInput::Assembly => {
            return assemble_to_avbc(path, output);
        }
        CompileInput::Bytecode => {
            return Err("input is already bytecode".to_string());
        }
        CompileInput::Source => {}
    }

    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;

    let name = path.display().to_string();
    let src = Source::new(&name, &content);

    let tokens = Lexer::with_source(src.clone())
        .scan()
        .map_err(|err| err.to_string())?;
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .map_err(|err| err.to_string())?;

    let mut vm = VM::with_config_and_args(src.clone(), VmConfig::default(), Vec::new())
        .map_err(|err| err.to_string())?;
    if let Ok(abs_path) = path.canonicalize() {
        vm.set_script_path(abs_path.display().to_string());
    } else {
        vm.set_script_path(path.display().to_string());
    }

    let (imports, loader) = load_modules_with_loader(&stmts, path, src.clone(), &mut vm)
        .map_err(|err| err.to_string())?;

    let main_stmts: Vec<_> = stmts
        .into_iter()
        .filter(|stmt| !matches!(stmt.kind, StmtKind::Needs(_)))
        .collect();

    let mut all_known_globals = imports.known_globals.clone();
    for builtin in BUILTIN_NAMES {
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
    )
    .compile_typed(&typed_program)
    .map_err(|err| err.to_string())?;

    let manifest_bytes = loader.manifest().map(Manifest::to_bytes);
    let should_bundle = loader
        .manifest()
        .map(|m| m.should_bundle_natives())
        .unwrap_or(false);

    let bytes = if should_bundle && !loader.loaded_native_modules().is_empty() {
        let bundles = build_native_bundles(loader.loaded_native_modules())?;
        aelys_bytecode::asm::serialize_with_manifest(
            &function,
            &heap,
            manifest_bytes.as_deref(),
            Some(&bundles),
        )
    } else if manifest_bytes.is_some() {
        aelys_bytecode::asm::serialize_with_manifest(
            &function,
            &heap,
            manifest_bytes.as_deref(),
            None,
        )
    } else {
        aelys_bytecode::asm::serialize(&function, &heap)
    };

    let output_path = output.unwrap_or_else(|| output_path_for(path));
    std::fs::write(&output_path, bytes)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;

    Ok(output_path)
}

pub fn run_with_options(
    path: &str,
    output: Option<String>,
    opt_level: OptimizationLevel,
) -> Result<i32, String> {
    let output = output.map(PathBuf::from);
    let output = compile_to_avbc_with_output(Path::new(path), output, opt_level)?;
    eprintln!("Wrote {}", output.display());
    Ok(0)
}

fn output_path_for(path: &Path) -> PathBuf {
    let mut output = path.to_path_buf();
    output.set_extension("avbc");
    output
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompileInput {
    Source,
    Assembly,
    Bytecode,
}

fn detect_format(path: &Path) -> CompileInput {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "aasm" => CompileInput::Assembly,
        "avbc" => CompileInput::Bytecode,
        _ => CompileInput::Source,
    }
}

fn assemble_to_avbc(path: &Path, output: Option<PathBuf>) -> Result<PathBuf, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let (functions, heap) =
        aelys_bytecode::asm::assemble(&content).map_err(|err| err.to_string())?;
    if functions.is_empty() {
        return Err("no functions found in assembly file".to_string());
    }
    let function = reconstruct_function_hierarchy(functions);
    let bytes = aelys_bytecode::asm::serialize(&function, &heap);
    let output_path = output.unwrap_or_else(|| output_path_for(path));
    std::fs::write(&output_path, bytes)
        .map_err(|err| format!("failed to write {}: {}", output_path.display(), err))?;
    Ok(output_path)
}

fn reconstruct_function_hierarchy(
    mut functions: Vec<aelys_bytecode::Function>,
) -> aelys_bytecode::Function {
    if functions.len() <= 1 {
        return functions
            .into_iter()
            .next()
            .unwrap_or_else(|| aelys_bytecode::Function::new(None, 0));
    }

    let mut main_func = functions.remove(0);
    main_func.nested_functions = functions;
    main_func
}

// bundle native modules into the .avbc for distribution
fn build_native_bundles(
    modules: &std::collections::HashMap<String, LoadedNativeInfo>,
) -> Result<Vec<NativeBundle>, String> {
    let mut bundles = Vec::new();
    for (name, info) in modules {
        let bytes = std::fs::read(&info.file_path)
            .map_err(|err| format!("failed to read {}: {}", info.file_path.display(), err))?;
        let checksum = compute_simple_hash(&bytes);
        let target = current_target_triple();
        bundles.push(NativeBundle {
            name: name.clone(),
            target,
            checksum,
            bytes,
        });
    }
    Ok(bundles)
}

fn compute_simple_hash(data: &[u8]) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

fn current_target_triple() -> String {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu".to_string()
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu".to_string()
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin".to_string()
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin".to_string()
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc".to_string()
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "unknown".to_string()
    }
}
