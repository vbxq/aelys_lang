use crate::modules::load_modules_with_loader;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::{Source, StmtKind};
use std::path::{Path, PathBuf};

const BUILTIN_NAMES: &[&str] = &["alloc", "free", "load", "store", "type"];

pub fn lower_file_to_air(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<aelys_air::AirProgram, String> {
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

    let (imports, _) = load_modules_with_loader(&stmts, path, src.clone(), &mut vm)
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
        src,
        imports.module_aliases,
        all_known_globals,
    )
    .map_err(|errors| {
        errors
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Unknown type error".to_string())
    })?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(typed_program);

    let mut air = aelys_air::lower::lower(&typed_program);
    aelys_air::layout::compute_layouts(&mut air);
    Ok(aelys_air::mono::monomorphize(air))
}

pub fn compile_file_with_llvm(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
) -> Result<(), String> {
    let air = lower_file_to_air(path, opt_level)?;
    compile_air_with_llvm(path, &air, emit_llvm_ir)
}

#[cfg(feature = "llvm-backend")]
fn compile_air_with_llvm(
    path: &Path,
    air: &aelys_air::AirProgram,
    emit_llvm_ir: bool,
) -> Result<(), String> {
    let module_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aelys_module");
    let mut codegen = aelys_codegen::CodegenContext::new(module_name);

    codegen.compile(air).map_err(|err| format!("{:?}", err))?;

    if emit_llvm_ir {
        let mut ir_path = PathBuf::from(path);
        ir_path.set_extension("ll");
        let ir_path_str = ir_path.to_string_lossy().to_string();
        codegen
            .emit_ir(&ir_path_str)
            .map_err(|err| format!("{:?}", err))?;
    }

    Ok(())
}

#[cfg(not(feature = "llvm-backend"))]
fn compile_air_with_llvm(
    _path: &Path,
    _air: &aelys_air::AirProgram,
    _emit_llvm_ir: bool,
) -> Result<(), String> {
    Err("LLVM backend is not enabled in this build of aelys-driver!".to_string())
}
