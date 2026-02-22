// source -> avbc compiler

use crate::cli::args::Backend;
use aelys_backend::Compiler;
use aelys_bytecode::asm::NativeBundle;
use aelys_common::{Warning, WarningConfig};
use aelys_driver::modules::{LoadedNativeInfo, load_modules_with_loader};
use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_modules::manifest::Manifest;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::{Source, StmtKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const BUILTIN_NAMES: &[&str] = &["alloc", "free", "load", "store", "type"];

#[allow(dead_code)]
pub fn compile_to_avbc(path: &Path, opt_level: OptimizationLevel) -> Result<PathBuf, String> {
    compile_to_avbc_with_output(path, None, opt_level, None).map(|r| r.output_path)
}

pub struct CompileResult {
    pub output_path: PathBuf,
    pub warnings: Vec<Warning>,
}

pub fn compile_to_avbc_with_output(
    path: &Path,
    output: Option<PathBuf>,
    opt_level: OptimizationLevel,
    source_for_warnings: Option<Arc<Source>>,
) -> Result<CompileResult, String> {
    match detect_format(path) {
        CompileInput::Assembly => {
            let out = assemble_to_avbc(path, output)?;
            return Ok(CompileResult {
                output_path: out,
                warnings: Vec::new(),
            });
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

    let warnings: Vec<Warning> = optimizer
        .take_warnings()
        .into_iter()
        .map(|mut w| {
            if w.source.is_none() {
                w.source = source_for_warnings.clone().or_else(|| Some(src.clone()));
            }
            w
        })
        .collect();

    let (mut function, heap, _globals) = Compiler::with_modules(
        None,
        src.clone(),
        imports.module_aliases,
        imports.known_globals,
        imports.known_native_globals,
        imports.symbol_origins,
    )
    .compile_typed(&typed_program)
    .map_err(|err| err.to_string())?;

    // strip debug info (function names, variable names, line info) for release builds
    if opt_level != OptimizationLevel::None {
        function.strip_debug_info();
    }

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

    Ok(CompileResult {
        output_path,
        warnings,
    })
}

pub fn run_with_options(
    path: &str,
    output: Option<String>,
    opt_level: OptimizationLevel,
    warn_config: WarningConfig,
    backend: Backend,
    emit_air: bool,
    emit_llvm_ir: bool,
) -> Result<i32, String> {
    match backend {
        Backend::Vm => {
            if emit_llvm_ir {
                return Err("--emit-llvm-ir requires --backend llvm".to_string());
            }

            if emit_air {
                return emit_air_program(path, opt_level);
            }

            let output = output.map(PathBuf::from);
            let result = compile_to_avbc_with_output(Path::new(path), output, opt_level, None)?;

            for w in &result.warnings {
                if warn_config.is_enabled(&w.kind) {
                    eprintln!("{}", w);
                }
            }

            if warn_config.treat_as_error && !result.warnings.is_empty() {
                let count = result.warnings.len();
                return Err(format!(
                    "aborting due to {} warning{}",
                    count,
                    if count == 1 { "" } else { "s" }
                ));
            }

            eprintln!("Wrote {}", result.output_path.display());
            Ok(0)
        }
        Backend::Llvm => {
            if emit_air {
                return Err("--emit-air is only supported with --backend vm".to_string());
            }
            if output.is_some() {
                return Err("--output is not supported with --backend llvm yet".to_string());
            }
            compile_file_with_llvm(Path::new(path), opt_level, emit_llvm_ir)?;
            if emit_llvm_ir {
                let mut ir_path = PathBuf::from(path);
                ir_path.set_extension("ll");
                eprintln!("Wrote {}", ir_path.display());
            }
            Ok(0)
        }
    }
}

pub fn emit_air_program(path: &str, opt_level: OptimizationLevel) -> Result<i32, String> {
    let path = Path::new(path);
    let air = lower_file_to_air(path, opt_level)?;
    print!("{}", aelys_air::print::print_program(&air));
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
